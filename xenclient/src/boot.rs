use crate::mem::PhysicalPages;
use crate::sys::{
    GrantEntry, SUPERPAGE_2MB_NR_PFNS, SUPERPAGE_2MB_SHIFT, SUPERPAGE_BATCH_SIZE, VGCF_IN_KERNEL,
    VGCF_ONLINE, XEN_PAGE_SHIFT,
};
use crate::x86::{
    PageTable, PageTableMapping, SharedInfo, StartInfo, MAX_GUEST_CMDLINE, X86_GUEST_MAGIC,
    X86_PAGE_SHIFT, X86_PAGE_SIZE, X86_PAGE_TABLE_MAX_MAPPINGS, X86_PGTABLE_LEVELS,
    X86_PGTABLE_LEVEL_SHIFT, X86_VIRT_MASK,
};
use crate::XenClientError;
use libc::{c_char, munmap};
use log::{debug, trace};
use slice_copy::copy;
use std::cmp::{max, min};
use std::ffi::c_void;
use std::mem::size_of;
use std::slice;
use xencall::domctl::DomainControl;
use xencall::memory::MemoryControl;
use xencall::sys::{VcpuGuestContext, MMUEXT_PIN_L4_TABLE};
use xencall::XenCall;

pub trait BootImageLoader {
    fn parse(&self) -> Result<BootImageInfo, XenClientError>;
    fn load(&self, image_info: &BootImageInfo, dst: &mut [u8]) -> Result<(), XenClientError>;
}

pub const XEN_UNSET_ADDR: u64 = -1i64 as u64;

#[derive(Debug)]
pub struct BootImageInfo {
    pub start: u64,
    pub virt_base: u64,
    pub virt_kstart: u64,
    pub virt_kend: u64,
    pub virt_hypercall: u64,
    pub virt_entry: u64,
    pub virt_p2m_base: u64,
    pub unmapped_initrd: bool,
}

pub struct BootSetup<'a> {
    domctl: &'a DomainControl<'a>,
    memctl: &'a MemoryControl<'a>,
    phys: PhysicalPages<'a>,
    domid: u32,
    virt_alloc_end: u64,
    pfn_alloc_end: u64,
    virt_pgtab_end: u64,
    total_pages: u64,
}

#[derive(Debug)]
pub struct DomainSegment {
    vstart: u64,
    _vend: u64,
    pfn: u64,
    addr: u64,
    size: u64,
    pages: u64,
}

#[derive(Debug)]
struct VmemRange {
    start: u64,
    end: u64,
    _flags: u32,
    _nid: u32,
}

#[derive(Debug)]
pub struct BootState {
    pub kernel_segment: DomainSegment,
    pub start_info_segment: DomainSegment,
    pub xenstore_segment: DomainSegment,
    pub console_segment: DomainSegment,
    pub boot_stack_segment: DomainSegment,
    pub p2m_segment: DomainSegment,
    pub page_table_segment: DomainSegment,
    pub page_table: PageTable,
    pub image_info: BootImageInfo,
    pub shared_info_frame: u64,
    pub initrd_segment: DomainSegment,
}

impl BootSetup<'_> {
    pub fn new<'a>(
        call: &'a XenCall,
        domctl: &'a DomainControl<'a>,
        memctl: &'a MemoryControl<'a>,
        domid: u32,
    ) -> BootSetup<'a> {
        BootSetup {
            domctl,
            memctl,
            phys: PhysicalPages::new(call, domid),
            domid,
            virt_alloc_end: 0,
            pfn_alloc_end: 0,
            virt_pgtab_end: 0,
            total_pages: 0,
        }
    }

    fn initialize_memory(&mut self, total_pages: u64) -> Result<(), XenClientError> {
        self.domctl.set_address_size(self.domid, 64)?;

        let mut vmemranges: Vec<VmemRange> = Vec::new();
        let stub = VmemRange {
            start: 0,
            end: total_pages << XEN_PAGE_SHIFT,
            _flags: 0,
            _nid: 0,
        };
        vmemranges.push(stub);
        debug!("BootSetup initialize_memory vmemranges: {:?}", vmemranges);

        let mut p2m_size: u64 = 0;
        let mut total: u64 = 0;
        for range in &vmemranges {
            total += (range.end - range.start) >> XEN_PAGE_SHIFT;
            p2m_size = p2m_size.max(range.end >> XEN_PAGE_SHIFT);
        }

        if total != total_pages {
            return Err(XenClientError::new(
                "page count mismatch while calculating pages",
            ));
        }

        debug!("BootSetup initialize_memory total_pages={}", total);
        self.total_pages = total;

        let mut p2m = vec![u64::MAX; p2m_size as usize];
        for range in &vmemranges {
            let mut extents_init = vec![0u64; SUPERPAGE_BATCH_SIZE as usize];
            let pages = (range.end - range.start) >> XEN_PAGE_SHIFT;
            let pfn_base = range.start >> XEN_PAGE_SHIFT;

            for pfn in pfn_base..pfn_base + pages {
                p2m[pfn as usize] = pfn;
            }

            let mut super_pages = pages >> SUPERPAGE_2MB_SHIFT;
            let mut pfn_base_idx: u64 = pfn_base;
            while super_pages > 0 {
                let count = super_pages.min(SUPERPAGE_BATCH_SIZE);
                super_pages -= count;

                let mut j: usize = 0;
                let mut pfn: u64 = pfn_base_idx;
                loop {
                    if pfn >= pfn_base_idx + (count << SUPERPAGE_2MB_SHIFT) {
                        break;
                    }
                    extents_init[j] = p2m[pfn as usize];
                    pfn += SUPERPAGE_2MB_NR_PFNS;
                    j += 1;
                }

                let extents_init_slice = extents_init.as_slice();
                let extents = self.memctl.populate_physmap(
                    self.domid,
                    count,
                    SUPERPAGE_2MB_SHIFT as u32,
                    0,
                    &extents_init_slice[0usize..count as usize],
                )?;

                pfn = pfn_base_idx;
                for mfn in extents {
                    for k in 0..SUPERPAGE_2MB_NR_PFNS {
                        p2m[pfn as usize] = mfn + k;
                        pfn += 1;
                    }
                }
                pfn_base_idx = pfn;
            }

            let mut j = pfn_base_idx - pfn_base;
            loop {
                if j >= pages {
                    break;
                }

                let allocsz = (1024 * 1024).min(pages - j);
                let p2m_idx = (pfn_base + j) as usize;
                let p2m_end_idx = p2m_idx + allocsz as usize;
                let input_extent_starts = &p2m[p2m_idx..p2m_end_idx];
                let result =
                    self.memctl
                        .populate_physmap(self.domid, allocsz, 0, 0, input_extent_starts)?;

                if result.len() != allocsz as usize {
                    return Err(XenClientError::new(
                        format!(
                            "failed to populate physmap: wanted={} received={} input_extents={}",
                            allocsz,
                            result.len(),
                            input_extent_starts.len()
                        )
                        .as_str(),
                    ));
                }

                for (i, item) in result.iter().enumerate() {
                    let p = (pfn_base + j + i as u64) as usize;
                    let m = *item;
                    p2m[p] = m;
                }
                j += allocsz;
            }
        }

        self.phys.load_p2m(p2m);
        Ok(())
    }

    fn setup_hypercall_page(&mut self, image_info: &BootImageInfo) -> Result<(), XenClientError> {
        if image_info.virt_hypercall == XEN_UNSET_ADDR {
            return Ok(());
        }

        let pfn = (image_info.virt_hypercall - image_info.virt_base) >> X86_PAGE_SHIFT;
        let mfn = self.phys.p2m[pfn as usize];
        self.domctl.hypercall_init(self.domid, mfn)?;
        Ok(())
    }

    pub fn initialize(
        &mut self,
        image_loader: &dyn BootImageLoader,
        initrd: &[u8],
        max_vcpus: u32,
        mem_mb: u64,
    ) -> Result<BootState, XenClientError> {
        debug!(
            "BootSetup initialize max_vcpus={:?} mem_mb={:?}",
            max_vcpus, mem_mb
        );
        self.domctl.set_max_vcpus(self.domid, max_vcpus)?;
        self.domctl.set_max_mem(self.domid, mem_mb * 1024)?;

        let total_pages = mem_mb << (20 - X86_PAGE_SHIFT);
        self.initialize_memory(total_pages)?;

        let image_info = image_loader.parse()?;
        debug!("BootSetup initialize image_info={:?}", image_info);
        self.virt_alloc_end = image_info.virt_base;
        let kernel_segment = self.load_kernel_segment(image_loader, &image_info)?;
        let mut p2m_segment: Option<DomainSegment> = None;
        let mut page_table = PageTable::default();
        if image_info.virt_p2m_base >= image_info.virt_base
            || (image_info.virt_p2m_base & ((1 << X86_PAGE_SHIFT) - 1)) != 0
        {
            p2m_segment = Some(self.alloc_p2m_segment(&mut page_table, &image_info)?);
        }
        let start_info_segment = self.alloc_page()?;
        let xenstore_segment = self.alloc_page()?;
        let console_segment = self.alloc_page()?;
        let page_table_segment = self.alloc_page_tables(&mut page_table, &image_info)?;
        let boot_stack_segment = self.alloc_page()?;

        if self.virt_pgtab_end > 0 {
            self.alloc_padding_pages(self.virt_pgtab_end)?;
        }

        let mut initrd_segment: Option<DomainSegment> = None;
        if !image_info.unmapped_initrd {
            initrd_segment = Some(self.alloc_module(initrd)?);
        }
        if p2m_segment.is_none() {
            let mut segment = self.alloc_p2m_segment(&mut page_table, &image_info)?;
            segment.vstart = image_info.virt_p2m_base;
            p2m_segment = Some(segment);
        }
        let p2m_segment = p2m_segment.unwrap();

        if image_info.unmapped_initrd {
            initrd_segment = Some(self.alloc_module(initrd)?);
        }

        let initrd_segment = initrd_segment.unwrap();

        let state = BootState {
            kernel_segment,
            start_info_segment,
            xenstore_segment,
            console_segment,
            boot_stack_segment,
            p2m_segment,
            page_table_segment,
            page_table,
            image_info,
            initrd_segment,
            shared_info_frame: 0,
        };
        debug!("BootSetup initialize state={:?}", state);
        Ok(state)
    }

    pub fn boot(&mut self, state: &mut BootState, cmdline: &str) -> Result<(), XenClientError> {
        let domain_info = self.domctl.get_domain_info(self.domid)?;
        let shared_info_frame = domain_info.shared_info_frame;
        state.shared_info_frame = shared_info_frame;
        self.setup_page_tables(state)?;
        self.setup_start_info(state, cmdline)?;
        self.setup_hypercall_page(&state.image_info)?;

        let pg_pfn = state.page_table_segment.pfn;
        self.phys.unmap(pg_pfn)?;
        self.phys.unmap(state.p2m_segment.pfn)?;
        let pg_mfn = self.phys.p2m[pg_pfn as usize];
        self.memctl
            .mmuext(self.domid, MMUEXT_PIN_L4_TABLE, pg_mfn, 0)?;
        self.setup_shared_info(state.shared_info_frame)?;

        let mut vcpu = VcpuGuestContext::default();
        vcpu.user_regs.rip = state.image_info.virt_entry;
        vcpu.user_regs.rsp =
            state.image_info.virt_base + (state.boot_stack_segment.pfn + 1) * X86_PAGE_SIZE;
        vcpu.user_regs.rsi =
            state.image_info.virt_base + (state.start_info_segment.pfn) * X86_PAGE_SIZE;
        vcpu.user_regs.rflags = 1 << 9;
        vcpu.debugreg[6] = 0xffff0ff0;
        vcpu.debugreg[7] = 0x00000400;
        vcpu.flags = VGCF_IN_KERNEL | VGCF_ONLINE;
        let cr3_pfn = pg_mfn;
        debug!(
            "cr3: pfn {:#x} mfn {:#x}",
            state.page_table_segment.pfn, cr3_pfn
        );
        vcpu.ctrlreg[3] = cr3_pfn << 12;
        vcpu.user_regs.ds = 0x0;
        vcpu.user_regs.es = 0x0;
        vcpu.user_regs.fs = 0x0;
        vcpu.user_regs.gs = 0x0;
        vcpu.user_regs.ss = 0xe02b;
        vcpu.user_regs.cs = 0xe033;
        vcpu.kernel_ss = vcpu.user_regs.ss as u64;
        vcpu.kernel_sp = vcpu.user_regs.rsp;
        debug!("vcpu context: {:?}", vcpu);
        self.domctl.set_vcpu_context(self.domid, 0, &vcpu)?;
        self.phys.unmap_all()?;
        self.gnttab_seed(state)?;
        Ok(())
    }

    fn gnttab_seed(&mut self, state: &mut BootState) -> Result<(), XenClientError> {
        let console_gfn = self.phys.p2m[state.console_segment.pfn as usize];
        let xenstore_gfn = self.phys.p2m[state.xenstore_segment.pfn as usize];
        let addr = self
            .domctl
            .call
            .mmap(0, 1 << XEN_PAGE_SHIFT)
            .ok_or(XenClientError::new("failed to mmap for resource"))?;
        self.domctl
            .call
            .map_resource(self.domid, 1, 0, 0, 1, addr)?;
        let entries = unsafe { slice::from_raw_parts_mut(addr as *mut GrantEntry, 2) };
        entries[0].flags = 1 << 0;
        entries[0].domid = 0;
        entries[0].frame = console_gfn as u32;
        entries[1].flags = 1 << 0;
        entries[1].domid = 0;
        entries[1].frame = xenstore_gfn as u32;
        unsafe {
            let result = munmap(addr as *mut c_void, 1 << XEN_PAGE_SHIFT);
            if result != 0 {
                return Err(XenClientError::new("failed to unmap resource"));
            }
        }
        Ok(())
    }

    fn setup_page_tables(&mut self, state: &mut BootState) -> Result<(), XenClientError> {
        let p2m_guest = unsafe {
            slice::from_raw_parts_mut(
                state.p2m_segment.addr as *mut u64,
                self.phys.p2m_size() as usize,
            )
        };
        copy(p2m_guest, &self.phys.p2m);

        for l in (0usize..X86_PGTABLE_LEVELS as usize).rev() {
            for m1 in 0usize..state.page_table.mappings_count {
                let map1 = &state.page_table.mappings[m1];
                let from = map1.levels[l].from;
                let to = map1.levels[l].to;
                let pg_ptr = self.phys.pfn_to_ptr(map1.levels[l].pfn, 0)? as *mut u64;
                for m2 in 0usize..state.page_table.mappings_count {
                    let map2 = &state.page_table.mappings[m2];
                    let lvl = if l > 0 {
                        &map2.levels[l - 1]
                    } else {
                        &map2.area
                    };

                    if l > 0 && lvl.pgtables == 0 {
                        continue;
                    }

                    if lvl.from >= to || lvl.to <= from {
                        continue;
                    }

                    let p_s = (max(from, lvl.from) - from)
                        >> (X86_PAGE_SHIFT + l as u64 * X86_PGTABLE_LEVEL_SHIFT);
                    let p_e = (min(to, lvl.to) - from)
                        >> (X86_PAGE_SHIFT + l as u64 * X86_PGTABLE_LEVEL_SHIFT);
                    let rhs = X86_PAGE_SHIFT as usize + l * X86_PGTABLE_LEVEL_SHIFT as usize;
                    let mut pfn = ((max(from, lvl.from) - lvl.from) >> rhs) + lvl.pfn;

                    debug!(
                        "BootSetup setup_page_tables lvl={} map_1={} map_2={} pfn={:#x} p_s={:#x} p_e={:#x}",
                        l, m1, m2, pfn, p_s, p_e
                    );

                    let pg = unsafe { slice::from_raw_parts_mut(pg_ptr, (p_e + 1) as usize) };
                    for p in p_s..p_e + 1 {
                        let prot = self.get_pg_prot(l, pfn, &state.page_table);
                        let pfn_paddr = self.phys.p2m[pfn as usize] << X86_PAGE_SHIFT;
                        let value = pfn_paddr | prot;
                        pg[p as usize] = value;
                        pfn += 1;
                    }
                }
            }
        }
        Ok(())
    }

    const PAGE_PRESENT: u64 = 0x001;
    const PAGE_RW: u64 = 0x002;
    const PAGE_USER: u64 = 0x004;
    const PAGE_ACCESSED: u64 = 0x020;
    const PAGE_DIRTY: u64 = 0x040;
    fn get_pg_prot(&mut self, l: usize, pfn: u64, table: &PageTable) -> u64 {
        let prot = [
            BootSetup::PAGE_PRESENT | BootSetup::PAGE_RW | BootSetup::PAGE_ACCESSED,
            BootSetup::PAGE_PRESENT
                | BootSetup::PAGE_RW
                | BootSetup::PAGE_ACCESSED
                | BootSetup::PAGE_DIRTY
                | BootSetup::PAGE_USER,
            BootSetup::PAGE_PRESENT
                | BootSetup::PAGE_RW
                | BootSetup::PAGE_ACCESSED
                | BootSetup::PAGE_DIRTY
                | BootSetup::PAGE_USER,
            BootSetup::PAGE_PRESENT
                | BootSetup::PAGE_RW
                | BootSetup::PAGE_ACCESSED
                | BootSetup::PAGE_DIRTY
                | BootSetup::PAGE_USER,
        ];

        let prot = prot[l];
        if l > 0 {
            return prot;
        }

        for m in 0..table.mappings_count {
            let map = &table.mappings[m];
            let pfn_s = map.levels[(X86_PGTABLE_LEVELS - 1) as usize].pfn;
            let pfn_e = map.area.pgtables as u64 + pfn_s;
            if pfn >= pfn_s && pfn < pfn_e {
                return prot & !BootSetup::PAGE_RW;
            }
        }
        prot
    }

    fn setup_start_info(&mut self, state: &BootState, cmdline: &str) -> Result<(), XenClientError> {
        let ptr = self.phys.pfn_to_ptr(state.start_info_segment.pfn, 1)?;
        let byte_slice =
            unsafe { slice::from_raw_parts_mut(ptr as *mut u8, X86_PAGE_SIZE as usize) };
        byte_slice.fill(0);
        let info = ptr as *mut StartInfo;
        unsafe {
            for (i, c) in X86_GUEST_MAGIC.chars().enumerate() {
                (*info).magic[i] = c as c_char;
            }
            (*info).magic[X86_GUEST_MAGIC.len()] = 0 as c_char;
            (*info).nr_pages = self.total_pages;
            (*info).shared_info = state.shared_info_frame << X86_PAGE_SHIFT;
            (*info).pt_base = state.page_table_segment.vstart;
            (*info).nr_pt_frames = state.page_table.mappings[0].area.pgtables as u64;
            (*info).mfn_list = state.p2m_segment.vstart;
            (*info).first_p2m_pfn = state.p2m_segment.pfn;
            (*info).nr_p2m_frames = state.p2m_segment.pages;
            (*info).flags = 0;
            (*info).store_evtchn = 0;
            (*info).store_mfn = self.phys.p2m[state.xenstore_segment.pfn as usize];
            (*info).console.mfn = self.phys.p2m[state.console_segment.pfn as usize];
            (*info).console.evtchn = 0;
            (*info).mod_start = state.initrd_segment.vstart;
            (*info).mod_len = state.initrd_segment.size;
            for (i, c) in cmdline.chars().enumerate() {
                (*info).cmdline[i] = c as c_char;
            }
            (*info).cmdline[MAX_GUEST_CMDLINE - 1] = 0;
            trace!("BootSetup setup_start_info start_info={:?}", *info);
        }
        Ok(())
    }

    fn setup_shared_info(&mut self, shared_info_frame: u64) -> Result<(), XenClientError> {
        let info = self
            .phys
            .map_foreign_pages(shared_info_frame, X86_PAGE_SIZE)?
            as *mut SharedInfo;
        unsafe {
            let size = size_of::<SharedInfo>();
            let info_as_buff = slice::from_raw_parts_mut(info as *mut u8, size);
            info_as_buff.fill(0);
            for i in 0..32 {
                (*info).vcpu_info[i].evtchn_upcall_mask = 1;
            }
            trace!("BootSetup setup_shared_info shared_info={:?}", *info);
        }
        Ok(())
    }

    fn load_kernel_segment(
        &mut self,
        image_loader: &dyn BootImageLoader,
        image_info: &BootImageInfo,
    ) -> Result<DomainSegment, XenClientError> {
        let kernel_segment = self.alloc_segment(
            image_info.virt_kstart,
            image_info.virt_kend - image_info.virt_kstart,
        )?;
        let kernel_segment_ptr = kernel_segment.addr as *mut u8;
        let kernel_segment_slice =
            unsafe { slice::from_raw_parts_mut(kernel_segment_ptr, kernel_segment.size as usize) };
        image_loader.load(image_info, kernel_segment_slice)?;
        Ok(kernel_segment)
    }

    fn count_page_tables(
        &mut self,
        table: &mut PageTable,
        from: u64,
        to: u64,
        pfn: u64,
    ) -> Result<usize, XenClientError> {
        debug!("counting pgtables from={} to={} pfn={}", from, to, pfn);
        if table.mappings_count == X86_PAGE_TABLE_MAX_MAPPINGS {
            return Err(XenClientError::new("too many mappings"));
        }

        let m = table.mappings_count;

        let pfn_end = pfn + ((to - from) >> X86_PAGE_SHIFT);
        if pfn_end >= self.phys.p2m_size() {
            return Err(XenClientError::new("not enough memory for initial mapping"));
        }

        for idx in 0..table.mappings_count {
            if from < table.mappings[idx].area.to && to > table.mappings[idx].area.from {
                return Err(XenClientError::new("overlapping mappings"));
            }
        }
        let mut map = PageTableMapping::default();
        map.area.from = from & X86_VIRT_MASK;
        map.area.to = to & X86_VIRT_MASK;

        for l in (0usize..X86_PGTABLE_LEVELS as usize).rev() {
            map.levels[l].pfn = self.pfn_alloc_end + map.area.pgtables as u64;
            if l as u64 == X86_PGTABLE_LEVELS - 1 {
                if table.mappings_count == 0 {
                    map.levels[l].from = 0;
                    map.levels[l].to = X86_VIRT_MASK;
                    map.levels[l].pgtables = 1;
                    map.area.pgtables += 1;
                }
                continue;
            }

            let bits = X86_PAGE_SHIFT + (l + 1) as u64 * X86_PGTABLE_LEVEL_SHIFT;
            let mask = BootSetup::bits_to_mask(bits);
            map.levels[l].from = map.area.from & !mask;
            map.levels[l].to = map.area.to | mask;

            for cmp in &mut table.mappings[0..table.mappings_count] {
                if cmp.levels[l].from == cmp.levels[l].to {
                    continue;
                }

                if map.levels[l].from >= cmp.levels[l].from && map.levels[l].to <= cmp.levels[l].to
                {
                    map.levels[l].from = 0;
                    map.levels[l].to = 0;
                    break;
                }

                if map.levels[l].from >= cmp.levels[l].from
                    && map.levels[l].from <= cmp.levels[l].to
                {
                    map.levels[l].from = cmp.levels[l].to + 1;
                }

                if map.levels[l].to >= cmp.levels[l].from && map.levels[l].to <= cmp.levels[l].to {
                    map.levels[l].to = cmp.levels[l].from - 1;
                }
            }

            if map.levels[l].from < map.levels[l].to {
                map.levels[l].pgtables =
                    (((map.levels[l].to - map.levels[l].from) >> bits) + 1) as usize;
            }

            debug!(
                "BootSetup count_pgtables {:#x}/{}: {:#x} -> {:#x}, {} tables",
                mask, bits, map.levels[l].from, map.levels[l].to, map.levels[l].pgtables
            );
            map.area.pgtables += map.levels[l].pgtables;
        }
        table.mappings[m] = map;
        Ok(m)
    }

    fn alloc_p2m_segment(
        &mut self,
        page_table: &mut PageTable,
        image_info: &BootImageInfo,
    ) -> Result<DomainSegment, XenClientError> {
        let mut p2m_alloc_size =
            ((self.phys.p2m_size() * 8) + X86_PAGE_SIZE - 1) & !(X86_PAGE_SIZE - 1);
        let from = image_info.virt_p2m_base;
        let to = from + p2m_alloc_size - 1;
        let m = self.count_page_tables(page_table, from, to, self.pfn_alloc_end)?;

        let pgtables: usize;
        {
            let map = &mut page_table.mappings[m];
            map.area.pfn = self.pfn_alloc_end;
            for lvl_idx in 0..4 {
                map.levels[lvl_idx].pfn += p2m_alloc_size >> X86_PAGE_SHIFT;
            }
            pgtables = map.area.pgtables;
        }
        page_table.mappings_count += 1;
        p2m_alloc_size += (pgtables << X86_PAGE_SHIFT) as u64;
        let p2m_segment = self.alloc_segment(0, p2m_alloc_size)?;
        Ok(p2m_segment)
    }

    fn round_up(addr: u64, mask: u64) -> u64 {
        addr | mask
    }

    fn bits_to_mask(bits: u64) -> u64 {
        (1 << bits) - 1
    }

    fn alloc_page_tables(
        &mut self,
        table: &mut PageTable,
        image_info: &BootImageInfo,
    ) -> Result<DomainSegment, XenClientError> {
        let mut extra_pages = 1;
        extra_pages += (512 * 1024) / X86_PAGE_SIZE;
        let mut pages = extra_pages;

        let mut try_virt_end: u64;
        let mut m: usize;
        loop {
            try_virt_end = BootSetup::round_up(
                self.virt_alloc_end + pages * X86_PAGE_SIZE,
                BootSetup::bits_to_mask(22),
            );
            m = self.count_page_tables(table, image_info.virt_base, try_virt_end, 0)?;
            pages = table.mappings[m].area.pgtables as u64 + extra_pages;
            if self.virt_alloc_end + pages * X86_PAGE_SIZE <= try_virt_end + 1 {
                break;
            }
        }

        table.mappings[m].area.pfn = 0;
        table.mappings_count += 1;
        self.virt_pgtab_end = try_virt_end + 1;
        let segment =
            self.alloc_segment(0, table.mappings[m].area.pgtables as u64 * X86_PAGE_SIZE)?;
        debug!(
            "BootSetup alloc_page_tables table={:?} segment={:?}",
            table, segment
        );
        Ok(segment)
    }

    fn alloc_segment(&mut self, start: u64, size: u64) -> Result<DomainSegment, XenClientError> {
        if start > 0 {
            self.alloc_padding_pages(start)?;
        }

        let page_size: u32 = (1i64 << XEN_PAGE_SHIFT) as u32;
        let pages = (size + page_size as u64 - 1) / page_size as u64;
        let start = self.virt_alloc_end;

        let mut segment = DomainSegment {
            vstart: start,
            _vend: 0,
            pfn: self.pfn_alloc_end,
            addr: 0,
            size,
            pages,
        };

        self.chk_alloc_pages(pages)?;

        let ptr = self.phys.pfn_to_ptr(segment.pfn, pages)?;
        segment.addr = ptr;
        let slice = unsafe {
            slice::from_raw_parts_mut(ptr as *mut u8, (pages * page_size as u64) as usize)
        };
        slice.fill(0);
        segment._vend = self.virt_alloc_end;
        debug!(
            "BootSetup alloc_segment {:#x} -> {:#x} (pfn {:#x} + {:#x} pages)",
            start, segment._vend, segment.pfn, pages
        );
        Ok(segment)
    }

    fn alloc_page(&mut self) -> Result<DomainSegment, XenClientError> {
        let start = self.virt_alloc_end;
        let pfn = self.pfn_alloc_end;

        self.chk_alloc_pages(1)?;
        debug!("BootSetup alloc_page {:#x} (pfn {:#x})", start, pfn);
        Ok(DomainSegment {
            vstart: start,
            _vend: (start + X86_PAGE_SIZE) - 1,
            pfn,
            addr: 0,
            size: 0,
            pages: 1,
        })
    }

    fn alloc_module(&mut self, buffer: &[u8]) -> Result<DomainSegment, XenClientError> {
        let segment = self.alloc_segment(0, buffer.len() as u64)?;
        let slice = unsafe { slice::from_raw_parts_mut(segment.addr as *mut u8, buffer.len()) };
        copy(slice, buffer);
        Ok(segment)
    }

    fn alloc_padding_pages(&mut self, boundary: u64) -> Result<(), XenClientError> {
        if (boundary & (X86_PAGE_SIZE - 1)) != 0 {
            return Err(XenClientError::new(
                format!("segment boundary isn't page aligned: {:#x}", boundary).as_str(),
            ));
        }

        if boundary < self.virt_alloc_end {
            return Err(XenClientError::new(
                format!("segment boundary too low: {:#x})", boundary).as_str(),
            ));
        }
        let pages = (boundary - self.virt_alloc_end) / X86_PAGE_SIZE;
        self.chk_alloc_pages(pages)?;
        Ok(())
    }

    fn chk_alloc_pages(&mut self, pages: u64) -> Result<(), XenClientError> {
        if pages > self.total_pages
            || self.pfn_alloc_end > self.total_pages
            || pages > self.total_pages - self.pfn_alloc_end
        {
            return Err(XenClientError::new(
                format!(
                    "segment too large: pages={} total_pages={} pfn_alloc_end={}",
                    pages, self.total_pages, self.pfn_alloc_end
                )
                .as_str(),
            ));
        }

        self.pfn_alloc_end += pages;
        self.virt_alloc_end += pages * X86_PAGE_SIZE;
        Ok(())
    }
}
