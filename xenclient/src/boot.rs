use crate::mem::PhysicalPages;
use crate::sys::{
    SUPERPAGE_2MB_NR_PFNS, SUPERPAGE_2MB_SHIFT, SUPERPAGE_BATCH_SIZE, XEN_PAGE_SHIFT,
};
use crate::x86::{
    PageTable, PageTableMapping, StartInfo, MAX_GUEST_CMDLINE, X86_GUEST_MAGIC, X86_PAGE_SHIFT,
    X86_PAGE_SIZE, X86_PAGE_TABLE_MAX_MAPPINGS, X86_PGTABLE_LEVELS, X86_PGTABLE_LEVEL_SHIFT,
    X86_VIRT_MASK,
};
use crate::XenClientError;
use libc::{c_char, memset};
use log::debug;
use std::cmp::{max, min};
use std::ffi::c_void;
use std::slice;
use xencall::domctl::DomainControl;
use xencall::memory::MemoryControl;
use xencall::sys::VcpuGuestContext;
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
    pub init_p2m: u64,
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
    _pages: u64,
}

struct VmemRange {
    start: u64,
    end: u64,
    _flags: u32,
    _nid: u32,
}

pub struct BootState {
    pub kernel_segment: DomainSegment,
    pub start_info_segment: DomainSegment,
    pub xenstore_segment: DomainSegment,
    pub console_segment: DomainSegment,
    pub boot_stack_segment: DomainSegment,
    pub page_table_segment: DomainSegment,
    pub page_table: PageTable,
    pub image_info: BootImageInfo,
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

    fn initialize_memory(&mut self, memkb: u64) -> Result<(), XenClientError> {
        self.domctl.set_address_size(self.domid, 64)?;

        let mem_mb: u64 = memkb / 1024;
        let page_count: u64 = mem_mb << (20 - XEN_PAGE_SHIFT);
        let mut vmemranges: Vec<VmemRange> = Vec::new();
        let stub = VmemRange {
            start: 0,
            end: page_count << XEN_PAGE_SHIFT,
            _flags: 0,
            _nid: 0,
        };
        vmemranges.push(stub);

        let mut p2m_size: u64 = 0;
        let mut total: u64 = 0;
        for range in &vmemranges {
            total += (range.end - range.start) >> XEN_PAGE_SHIFT;
            p2m_size = p2m_size.max(range.end >> XEN_PAGE_SHIFT);
        }

        if total != page_count {
            return Err(XenClientError::new(
                "Page count mismatch while calculating pages.",
            ));
        }

        self.total_pages = total;

        let mut p2m = vec![-1i64 as u64; p2m_size as usize];
        for range in &vmemranges {
            let mut extents = vec![0u64; SUPERPAGE_BATCH_SIZE as usize];
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
                    extents[j] = p2m[pfn as usize];
                    pfn += SUPERPAGE_2MB_NR_PFNS;
                    j += 1;
                }

                let starts = self.memctl.populate_physmap(
                    self.domid,
                    count,
                    SUPERPAGE_2MB_SHIFT as u32,
                    0,
                    extents.as_slice(),
                )?;

                pfn = pfn_base_idx;
                for mfn in starts {
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
                let result = self.memctl.populate_physmap(
                    self.domid,
                    allocsz,
                    0,
                    0,
                    &p2m[p2m_idx..p2m_end_idx],
                )?;

                if result.len() != allocsz as usize {
                    return Err(XenClientError::new(
                        format!("failed to populate physmap: {:?}", result).as_str(),
                    ));
                }

                p2m[p2m_idx] = result[0];
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
        memkb: u64,
    ) -> Result<BootState, XenClientError> {
        debug!("BootSetup initialize memkb={:?}", memkb);
        self.domctl.set_max_mem(self.domid, memkb)?;
        self.initialize_memory(memkb)?;

        let image_info = image_loader.parse()?;
        self.virt_alloc_end = image_info.virt_base;
        let kernel_segment = self.load_kernel_segment(image_loader, &image_info)?;
        let start_info_segment = self.alloc_page()?;
        let xenstore_segment = self.alloc_page()?;
        let console_segment = self.alloc_page()?;
        let (page_table_segment, page_table) = self.alloc_page_tables(&image_info)?;
        let boot_stack_segment = self.alloc_page()?;
        if self.virt_pgtab_end > 0 {
            self.alloc_padding_pages(self.virt_pgtab_end)?;
        }
        Ok(BootState {
            kernel_segment,
            start_info_segment,
            xenstore_segment,
            console_segment,
            boot_stack_segment,
            page_table_segment,
            page_table,
            image_info,
        })
    }

    pub fn boot(&mut self, state: &mut BootState, cmdline: &str) -> Result<(), XenClientError> {
        self.setup_page_tables(state)?;
        self.setup_start_info(state, cmdline);
        self.setup_hypercall_page(&state.image_info)?;

        let mut vcpu = VcpuGuestContext::default();
        vcpu.user_regs.rip = state.image_info.virt_entry;
        vcpu.user_regs.rsp =
            state.image_info.virt_base + (state.boot_stack_segment.pfn + 1) * X86_PAGE_SIZE;
        vcpu.user_regs.rsi =
            state.image_info.virt_base + (state.start_info_segment.pfn) * X86_PAGE_SIZE;
        vcpu.user_regs.rflags = 1 << 9;
        vcpu.debugreg[6] = 0xffff0ff0;
        vcpu.debugreg[7] = 0x00000400;
        vcpu.flags = (1 << 2) | (1 << 5);
        let cr3_pfn = self.phys.p2m[state.page_table_segment.pfn as usize];
        vcpu.ctrlreg[3] = cr3_pfn << 12;
        vcpu.user_regs.ds = 0xe021;
        vcpu.user_regs.es = 0xe021;
        vcpu.user_regs.fs = 0xe021;
        vcpu.user_regs.gs = 0xe021;
        vcpu.user_regs.ss = 0xe02b;
        vcpu.user_regs.cs = 0xe019;
        vcpu.kernel_ss = vcpu.user_regs.ss as u64;
        vcpu.kernel_sp = vcpu.user_regs.rsp;
        let _vcpu = vcpu;
        self.domctl.set_vcpu_context(self.domid, 0, Some(&vcpu))?;
        Ok(())
    }

    fn setup_page_tables(&mut self, state: &mut BootState) -> Result<(), XenClientError> {
        for lvl_idx in (0usize..3usize).rev() {
            for map_idx_1 in 0usize..state.page_table.mappings_count {
                let map1 = &state.page_table.mappings[map_idx_1];
                let from = map1.levels[lvl_idx].from;
                let to = map1.levels[lvl_idx].to;
                let pg = self.phys.pfn_to_ptr(map1.levels[lvl_idx].pfn, 0)? as *mut u64;
                for map_idx_2 in 0usize..state.page_table.mappings_count {
                    let map2 = &state.page_table.mappings[map_idx_2];
                    let lvl = if lvl_idx > 0 {
                        &map2.levels[lvl_idx - 1]
                    } else {
                        &map2.area
                    };

                    if lvl_idx > 0 && lvl.pgtables == 0 {
                        continue;
                    }

                    if lvl.from >= to || lvl.to <= from {
                        continue;
                    }

                    let p_s = (max(from, lvl.from) - from)
                        >> (X86_PAGE_SHIFT + lvl_idx as u64 * X86_PGTABLE_LEVEL_SHIFT);
                    let p_e = (min(to, lvl.to) - from)
                        >> (X86_PAGE_SHIFT + lvl_idx as u64 * X86_PGTABLE_LEVEL_SHIFT);
                    let mut pfn = (max(from, lvl.from) - from)
                        .checked_shr(
                            ((X86_PAGE_SHIFT + lvl_idx as u64 * X86_PGTABLE_LEVEL_SHIFT) + lvl.pfn)
                                as u32,
                        )
                        .unwrap_or(0u64);

                    for p in p_s..p_e + 1 {
                        unsafe {
                            *pg.add(p as usize) = self.phys.p2m[pfn as usize] << X86_PAGE_SHIFT;
                        }
                        pfn += 1;
                    }
                }
            }
        }
        Ok(())
    }

    fn setup_start_info(&mut self, state: &BootState, cmdline: &str) {
        let info = state.start_info_segment.addr as *mut StartInfo;
        unsafe {
            for (i, c) in X86_GUEST_MAGIC.chars().enumerate() {
                (*info).magic[i] = c as c_char;
            }
            (*info).nr_pages = self.total_pages;
            (*info).shared_info = 0;
            (*info).pt_base = state.page_table_segment.vstart;
            (*info).nr_pt_frames = state.page_table.mappings[0].area.pgtables as u64;
            (*info).mfn_list = 0;
            (*info).first_p2m_pfn = 0;
            (*info).nr_p2m_frames = 0;
            (*info).flags = 0;
            (*info).store_evtchn = 0;
            (*info).store_mfn = 0;
            (*info).console.mfn = self.phys.p2m[state.console_segment.pfn as usize];
            (*info).console.evtchn = 0;
            (*info).mod_start = 0;
            (*info).mod_len = 0;
            for (i, c) in cmdline.chars().enumerate() {
                (*info).cmdline[i] = c as c_char;
                (*info).cmdline[MAX_GUEST_CMDLINE - 1] = 0;
            }
            debug!("BootSetup setup_start_info={:?}", *info);
        }
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
        debug!(
            "BootSetup initialize kernel_segment ptr={:#x}",
            kernel_segment_ptr as u64
        );
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
    ) -> Result<(), XenClientError> {
        if table.mappings_count == X86_PAGE_TABLE_MAX_MAPPINGS {
            return Err(XenClientError::new("too many mappings"));
        }

        let pfn_end = pfn + ((to - from) >> X86_PAGE_SHIFT);
        if pfn_end >= self.phys.p2m_size() {
            return Err(XenClientError::new("not enough memory for initial mapping"));
        }

        for mapping in &table.mappings {
            if from < mapping.area.to && to > mapping.area.from {
                return Err(XenClientError::new("overlapping mappings"));
            }
        }

        table.mappings[table.mappings_count] = PageTableMapping::default();
        let compare_table = table.clone();
        let map = &mut table.mappings[table.mappings_count];
        map.area.from = from & X86_VIRT_MASK;
        map.area.to = to & X86_VIRT_MASK;

        for lvl_index in (0usize..3usize).rev() {
            let lvl = &mut map.levels[lvl_index];
            lvl.pfn = self.pfn_alloc_end + map.area.pgtables as u64;
            if lvl_index as u64 == X86_PGTABLE_LEVELS - 1 {
                if table.mappings_count == 0 {
                    lvl.from = 0;
                    lvl.to = X86_VIRT_MASK;
                    lvl.pgtables = 1;
                    map.area.pgtables += 1;
                }
                continue;
            }

            let bits = X86_PAGE_SHIFT + (lvl_index + 1) as u64 * X86_PGTABLE_LEVEL_SHIFT;
            let mask = (1 << bits) - 1;
            lvl.from = map.area.from & !mask;
            lvl.to = map.area.to | mask;

            for cmp in &compare_table.mappings {
                let cmp_lvl = &cmp.levels[lvl_index];
                if cmp_lvl.from == cmp_lvl.to {
                    continue;
                }

                if lvl.from >= cmp_lvl.from && lvl.to <= cmp_lvl.to {
                    lvl.from = 0;
                    lvl.to = 0;
                }

                if lvl.from >= cmp_lvl.from && lvl.from <= cmp_lvl.to {
                    lvl.from = cmp_lvl.to + 1;
                }

                if lvl.to >= cmp_lvl.from && lvl.to <= cmp_lvl.to {
                    lvl.to = cmp_lvl.from - 1;
                }
            }

            if lvl.from < lvl.to {
                lvl.pgtables = (((lvl.to - lvl.from) >> bits) + 1) as usize;
            }

            map.area.pgtables += lvl.pgtables;
        }
        Ok(())
    }

    fn alloc_page_tables(
        &mut self,
        image_info: &BootImageInfo,
    ) -> Result<(DomainSegment, PageTable), XenClientError> {
        let mut table = PageTable::default();
        let extra_pages = ((512 * 1024) / X86_PAGE_SIZE) + 1;
        let mut pages = extra_pages;

        let mut try_virt_end: u64;
        loop {
            try_virt_end = (self.virt_alloc_end + pages * X86_PAGE_SIZE) | ((1 << 22) - 1);
            self.count_page_tables(&mut table, image_info.virt_base, try_virt_end, 0)?;
            pages = table.mappings[0].area.pgtables as u64 + extra_pages;
            if self.virt_alloc_end + pages * X86_PAGE_SIZE <= try_virt_end + 1 {
                break;
            }
        }

        let segment: DomainSegment;
        {
            let map = &mut table.mappings[table.mappings_count];
            map.area.pfn = 0;
            table.mappings_count += 1;
            self.virt_pgtab_end = try_virt_end + 1;
            segment = self.alloc_segment(0, map.area.pgtables as u64 * X86_PAGE_SIZE)?;
        }
        debug!(
            "BootSetup alloc_page_tables table={:?} segment={:?}",
            table, segment
        );
        Ok((segment, table))
    }

    fn alloc_segment(&mut self, start: u64, size: u64) -> Result<DomainSegment, XenClientError> {
        if start > 0 {
            self.alloc_padding_pages(start)?;
        }

        let start = self.virt_alloc_end;
        let page_size = 1u64 << XEN_PAGE_SHIFT;
        let pages = (size + page_size - 1) / page_size;

        let mut segment = DomainSegment {
            vstart: start,
            _vend: 0,
            pfn: self.pfn_alloc_end,
            addr: 0,
            size,
            _pages: pages,
        };

        self.chk_alloc_pages(pages)?;

        let ptr = self.phys.pfn_to_ptr(segment.pfn, pages)?;
        segment.addr = ptr;
        unsafe {
            memset(ptr as *mut c_void, 0, (pages * page_size) as usize);
        }
        segment._vend = self.virt_alloc_end;
        debug!(
            "BootSetup alloc_segment start={:#x} size={} ptr={:#x}",
            start, size, ptr
        );
        Ok(segment)
    }

    fn alloc_page(&mut self) -> Result<DomainSegment, XenClientError> {
        let page_size = 1u64 << XEN_PAGE_SHIFT;
        self.alloc_segment(0, page_size)
    }

    fn alloc_padding_pages(&mut self, boundary: u64) -> Result<(), XenClientError> {
        if (boundary & (X86_PAGE_SIZE - 1)) != 0 {
            return Err(XenClientError::new(
                format!("segment boundary isn't page aligned: {:#x}", boundary).as_str(),
            ));
        }

        if boundary < self.virt_alloc_end {
            return Err(XenClientError::new("segment boundary too low"));
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
