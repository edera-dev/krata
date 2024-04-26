use crate::boot::{ArchBootSetup, BootImageInfo, BootSetup, BootState, DomainSegment};
use crate::error::Result;
use crate::sys::{
    SUPERPAGE_2MB_NR_PFNS, SUPERPAGE_2MB_SHIFT, SUPERPAGE_BATCH_SIZE, VGCF_IN_KERNEL, VGCF_ONLINE,
    XEN_PAGE_SHIFT,
};
use crate::Error;
use libc::c_char;
use log::{debug, trace};
use slice_copy::copy;
use std::cmp::{max, min};
use std::mem::size_of;
use std::slice;
use xencall::sys::{
    E820Entry, VcpuGuestContext, E820_MAX, E820_RAM, E820_UNUSABLE, MMUEXT_PIN_L4_TABLE,
};

pub const X86_PAGE_SHIFT: u64 = 12;
pub const X86_PAGE_SIZE: u64 = 1 << X86_PAGE_SHIFT;
pub const X86_VIRT_BITS: u64 = 48;
pub const X86_VIRT_MASK: u64 = (1 << X86_VIRT_BITS) - 1;
pub const X86_PGTABLE_LEVELS: u64 = 4;
pub const X86_PGTABLE_LEVEL_SHIFT: u64 = 9;

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct PageTableMappingLevel {
    pub from: u64,
    pub to: u64,
    pub pfn: u64,
    pub pgtables: usize,
}

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct PageTableMapping {
    pub area: PageTableMappingLevel,
    pub levels: [PageTableMappingLevel; X86_PGTABLE_LEVELS as usize],
}

pub const X86_PAGE_TABLE_MAX_MAPPINGS: usize = 2;

#[repr(C)]
#[derive(Debug, Clone, Default)]
pub struct PageTable {
    pub mappings_count: usize,
    pub mappings: [PageTableMapping; X86_PAGE_TABLE_MAX_MAPPINGS],
}

#[repr(C)]
#[derive(Debug)]
pub struct StartInfoConsole {
    pub mfn: u64,
    pub evtchn: u32,
}

pub const MAX_GUEST_CMDLINE: usize = 1024;

#[repr(C)]
#[derive(Debug)]
pub struct StartInfo {
    pub magic: [c_char; 32],
    pub nr_pages: u64,
    pub shared_info: u64,
    pub flags: u32,
    pub store_mfn: u64,
    pub store_evtchn: u32,
    pub console: StartInfoConsole,
    pub pt_base: u64,
    pub nr_pt_frames: u64,
    pub mfn_list: u64,
    pub mod_start: u64,
    pub mod_len: u64,
    pub cmdline: [c_char; MAX_GUEST_CMDLINE],
    pub first_p2m_pfn: u64,
    pub nr_p2m_frames: u64,
}

pub const X86_GUEST_MAGIC: &str = "xen-3.0-x86_64";

#[repr(C)]
#[derive(Debug)]
pub struct ArchVcpuInfo {
    pub cr2: u64,
    pub pad: u64,
}

#[repr(C)]
#[derive(Debug)]
pub struct VcpuInfoTime {
    pub version: u32,
    pub pad0: u32,
    pub tsc_timestamp: u64,
    pub system_time: u64,
    pub tsc_to_system_mul: u32,
    pub tsc_shift: i8,
    pub flags: u8,
    pub pad1: [u8; 2],
}

#[repr(C)]
#[derive(Debug)]
pub struct VcpuInfo {
    pub evtchn_upcall_pending: u8,
    pub evtchn_upcall_mask: u8,
    pub evtchn_pending_sel: u64,
    pub arch_vcpu_info: ArchVcpuInfo,
    pub vcpu_info_time: VcpuInfoTime,
}

#[repr(C)]
#[derive(Debug)]
pub struct SharedInfo {
    pub vcpu_info: [VcpuInfo; 32],
    pub evtchn_pending: [u64; u64::BITS as usize],
    pub evtchn_mask: [u64; u64::BITS as usize],
    pub wc_version: u32,
    pub wc_sec: u32,
    pub wc_nsec: u32,
    pub wc_sec_hi: u32,
    // arch shared info
    pub max_pfn: u64,
    pub pfn_to_mfn_frame_list_list: u64,
    pub nmi_reason: u64,
    pub p2m_cr3: u64,
    pub p2m_vaddr: u64,
    pub p2m_generation: u64,
}

pub struct X86BootSetup {
    table: PageTable,
}

#[derive(Debug)]
struct VmemRange {
    start: u64,
    end: u64,
    _flags: u32,
    _nid: u32,
}

impl Default for X86BootSetup {
    fn default() -> Self {
        Self::new()
    }
}

impl X86BootSetup {
    pub fn new() -> X86BootSetup {
        X86BootSetup {
            table: PageTable::default(),
        }
    }

    const PAGE_PRESENT: u64 = 0x001;
    const PAGE_RW: u64 = 0x002;
    const PAGE_USER: u64 = 0x004;
    const PAGE_ACCESSED: u64 = 0x020;
    const PAGE_DIRTY: u64 = 0x040;
    fn get_pg_prot(&mut self, l: usize, pfn: u64) -> u64 {
        let prot = [
            X86BootSetup::PAGE_PRESENT | X86BootSetup::PAGE_RW | X86BootSetup::PAGE_ACCESSED,
            X86BootSetup::PAGE_PRESENT
                | X86BootSetup::PAGE_RW
                | X86BootSetup::PAGE_ACCESSED
                | X86BootSetup::PAGE_DIRTY
                | X86BootSetup::PAGE_USER,
            X86BootSetup::PAGE_PRESENT
                | X86BootSetup::PAGE_RW
                | X86BootSetup::PAGE_ACCESSED
                | X86BootSetup::PAGE_DIRTY
                | X86BootSetup::PAGE_USER,
            X86BootSetup::PAGE_PRESENT
                | X86BootSetup::PAGE_RW
                | X86BootSetup::PAGE_ACCESSED
                | X86BootSetup::PAGE_DIRTY
                | X86BootSetup::PAGE_USER,
        ];

        let prot = prot[l];
        if l > 0 {
            return prot;
        }

        for m in 0..self.table.mappings_count {
            let map = &self.table.mappings[m];
            let pfn_s = map.levels[(X86_PGTABLE_LEVELS - 1) as usize].pfn;
            let pfn_e = map.area.pgtables as u64 + pfn_s;
            if pfn >= pfn_s && pfn < pfn_e {
                return prot & !X86BootSetup::PAGE_RW;
            }
        }
        prot
    }

    fn count_page_tables(
        &mut self,
        setup: &mut BootSetup,
        from: u64,
        to: u64,
        pfn: u64,
    ) -> Result<usize> {
        debug!("counting pgtables from={} to={} pfn={}", from, to, pfn);
        if self.table.mappings_count == X86_PAGE_TABLE_MAX_MAPPINGS {
            return Err(Error::MemorySetupFailed("max page table count reached"));
        }

        let m = self.table.mappings_count;

        let pfn_end = pfn + ((to - from) >> X86_PAGE_SHIFT);
        if pfn_end >= setup.phys.p2m_size() {
            return Err(Error::MemorySetupFailed("pfn_end greater than p2m size"));
        }

        for idx in 0..self.table.mappings_count {
            if from < self.table.mappings[idx].area.to && to > self.table.mappings[idx].area.from {
                return Err(Error::MemorySetupFailed("page table calculation failed"));
            }
        }
        let mut map = PageTableMapping::default();
        map.area.from = from & X86_VIRT_MASK;
        map.area.to = to & X86_VIRT_MASK;

        for l in (0usize..X86_PGTABLE_LEVELS as usize).rev() {
            map.levels[l].pfn = setup.pfn_alloc_end + map.area.pgtables as u64;
            if l as u64 == X86_PGTABLE_LEVELS - 1 {
                if self.table.mappings_count == 0 {
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

            for cmp in &mut self.table.mappings[0..self.table.mappings_count] {
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
                "count_pgtables {:#x}/{}: {:#x} -> {:#x}, {} tables",
                mask, bits, map.levels[l].from, map.levels[l].to, map.levels[l].pgtables
            );
            map.area.pgtables += map.levels[l].pgtables;
        }
        self.table.mappings[m] = map;
        Ok(m)
    }

    fn e820_sanitize(
        &self,
        mut source: Vec<E820Entry>,
        map_limit_kb: u64,
        balloon_kb: u64,
    ) -> Result<Vec<E820Entry>> {
        let mut e820 = vec![E820Entry::default(); E820_MAX as usize];

        for entry in &mut source {
            if entry.addr > 0x100000 {
                continue;
            }

            // entries under 1MB should be removed.
            entry.typ = 0;
            entry.size = 0;
            entry.addr = u64::MAX;
        }

        let mut lowest = u64::MAX;
        let mut highest = 0;

        for entry in &source {
            if entry.typ == E820_RAM || entry.typ == E820_UNUSABLE || entry.typ == 0 {
                continue;
            }

            lowest = if entry.addr < lowest {
                entry.addr
            } else {
                lowest
            };

            highest = if entry.addr + entry.size > highest {
                entry.addr + entry.size
            } else {
                highest
            }
        }

        let start_kb = if lowest > 1024 { lowest >> 10 } else { 0 };

        let mut idx: usize = 0;

        e820[idx].addr = 0;
        e820[idx].size = map_limit_kb << 10;
        e820[idx].typ = E820_RAM;

        let mut delta_kb = 0u64;

        if start_kb > 0 && map_limit_kb > start_kb {
            delta_kb = map_limit_kb - start_kb;
            if delta_kb > 0 {
                e820[idx].size -= delta_kb << 10;
            }
        }

        let ram_end = source[0].addr + source[0].size;
        idx += 1;

        for src in &mut source {
            let end = src.addr + src.size;
            if src.typ == E820_UNUSABLE || end < ram_end {
                src.typ = 0;
                continue;
            }

            if src.typ != E820_RAM {
                continue;
            }

            if src.addr >= (1 << 32) {
                continue;
            }

            if src.addr < ram_end {
                let delta = ram_end - src.addr;
                src.typ = E820_UNUSABLE;

                if src.size < delta {
                    src.typ = 0;
                } else {
                    src.size -= delta;
                    src.addr = ram_end;
                }

                if src.addr + src.size != end {
                    src.typ = 0;
                }
            }

            if end > ram_end {
                src.typ = E820_UNUSABLE;
            }
        }

        if lowest > ram_end {
            let mut add_unusable = true;

            for src in &mut source {
                if !add_unusable {
                    break;
                }

                if src.typ != E820_UNUSABLE {
                    continue;
                }

                if ram_end != src.addr {
                    continue;
                }

                if lowest != src.addr + src.size {
                    src.size = lowest - src.addr;
                }
                add_unusable = false;
            }

            if add_unusable {
                e820[1].typ = E820_UNUSABLE;
                e820[1].addr = ram_end;
                e820[1].size = lowest - ram_end;
            }
        }

        for src in &source {
            if src.typ == E820_RAM || src.typ == 0 {
                continue;
            }

            e820[idx].typ = src.typ;
            e820[idx].addr = src.addr;
            e820[idx].size = src.size;
            idx += 1;
        }

        if balloon_kb > 0 || delta_kb > 0 {
            e820[idx].typ = E820_RAM;
            e820[idx].addr = if (1u64 << 32u64) > highest {
                1u64 << 32u64
            } else {
                highest
            };
            e820[idx].size = (delta_kb << 10) + (balloon_kb << 10);
        }
        Ok(e820)
    }
}

#[async_trait::async_trait]
impl ArchBootSetup for X86BootSetup {
    fn page_size(&mut self) -> u64 {
        X86_PAGE_SIZE
    }

    fn page_shift(&mut self) -> u64 {
        X86_PAGE_SHIFT
    }

    fn needs_early_kernel(&mut self) -> bool {
        false
    }

    async fn alloc_p2m_segment(
        &mut self,
        setup: &mut BootSetup,
        image_info: &BootImageInfo,
    ) -> Result<Option<DomainSegment>> {
        let mut p2m_alloc_size =
            ((setup.phys.p2m_size() * 8) + X86_PAGE_SIZE - 1) & !(X86_PAGE_SIZE - 1);
        let from = image_info.virt_p2m_base;
        let to = from + p2m_alloc_size - 1;
        let m = self.count_page_tables(setup, from, to, setup.pfn_alloc_end)?;

        let pgtables: usize;
        {
            let map = &mut self.table.mappings[m];
            map.area.pfn = setup.pfn_alloc_end;
            for lvl_idx in 0..4 {
                map.levels[lvl_idx].pfn += p2m_alloc_size >> X86_PAGE_SHIFT;
            }
            pgtables = map.area.pgtables;
        }
        self.table.mappings_count += 1;
        p2m_alloc_size += (pgtables << X86_PAGE_SHIFT) as u64;
        let p2m_segment = setup
            .alloc_segment(self.page_size(), 0, p2m_alloc_size)
            .await?;
        Ok(Some(p2m_segment))
    }

    async fn alloc_page_tables(
        &mut self,
        setup: &mut BootSetup,
        image_info: &BootImageInfo,
    ) -> Result<Option<DomainSegment>> {
        let mut extra_pages = 1;
        extra_pages += (512 * 1024) / X86_PAGE_SIZE;
        let mut pages = extra_pages;

        let mut try_virt_end: u64;
        let mut m: usize;
        loop {
            try_virt_end = BootSetup::round_up(
                setup.virt_alloc_end + pages * X86_PAGE_SIZE,
                BootSetup::bits_to_mask(22),
            );
            m = self.count_page_tables(setup, image_info.virt_base, try_virt_end, 0)?;
            pages = self.table.mappings[m].area.pgtables as u64 + extra_pages;
            if setup.virt_alloc_end + pages * X86_PAGE_SIZE <= try_virt_end + 1 {
                break;
            }
        }

        self.table.mappings[m].area.pfn = 0;
        self.table.mappings_count += 1;
        setup.virt_pgtab_end = try_virt_end + 1;
        let size = self.table.mappings[m].area.pgtables as u64 * X86_PAGE_SIZE;
        let segment = setup.alloc_segment(self.page_size(), 0, size).await?;
        debug!(
            "alloc_page_tables table={:?} segment={:?}",
            self.table, segment
        );
        Ok(Some(segment))
    }

    async fn setup_page_tables(
        &mut self,
        setup: &mut BootSetup,
        state: &mut BootState,
    ) -> Result<()> {
        let p2m_segment = state
            .p2m_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("p2m_segment missing"))?;
        let p2m_guest = unsafe {
            slice::from_raw_parts_mut(p2m_segment.addr as *mut u64, setup.phys.p2m_size() as usize)
        };
        copy(p2m_guest, &setup.phys.p2m);

        for l in (0usize..X86_PGTABLE_LEVELS as usize).rev() {
            for m1 in 0usize..self.table.mappings_count {
                let map1 = &self.table.mappings[m1];
                let from = map1.levels[l].from;
                let to = map1.levels[l].to;
                let pg_ptr = setup.phys.pfn_to_ptr(map1.levels[l].pfn, 0).await? as *mut u64;
                for m2 in 0usize..self.table.mappings_count {
                    let map2 = &self.table.mappings[m2];
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
                        "setup_page_tables lvl={} map_1={} map_2={} pfn={:#x} p_s={:#x} p_e={:#x}",
                        l, m1, m2, pfn, p_s, p_e
                    );

                    let pg = unsafe { slice::from_raw_parts_mut(pg_ptr, (p_e + 1) as usize) };
                    for p in p_s..p_e + 1 {
                        let prot = self.get_pg_prot(l, pfn);
                        let pfn_paddr = setup.phys.p2m[pfn as usize] << X86_PAGE_SHIFT;
                        let value = pfn_paddr | prot;
                        pg[p as usize] = value;
                        pfn += 1;
                    }
                }
            }
        }
        Ok(())
    }

    async fn setup_start_info(
        &mut self,
        setup: &mut BootSetup,
        state: &BootState,
        cmdline: &str,
    ) -> Result<()> {
        let ptr = setup
            .phys
            .pfn_to_ptr(state.start_info_segment.pfn, 1)
            .await?;
        let byte_slice =
            unsafe { slice::from_raw_parts_mut(ptr as *mut u8, X86_PAGE_SIZE as usize) };
        byte_slice.fill(0);
        let info = ptr as *mut StartInfo;

        let page_table_segment = state
            .page_table_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("page_table_segment missing"))?;
        let p2m_segment = state
            .p2m_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("p2m_segment missing"))?;
        unsafe {
            for (i, c) in X86_GUEST_MAGIC.chars().enumerate() {
                (*info).magic[i] = c as c_char;
            }
            (*info).magic[X86_GUEST_MAGIC.len()] = 0 as c_char;
            (*info).nr_pages = setup.total_pages;
            (*info).shared_info = state.shared_info_frame << X86_PAGE_SHIFT;
            (*info).pt_base = page_table_segment.vstart;
            (*info).nr_pt_frames = self.table.mappings[0].area.pgtables as u64;
            (*info).mfn_list = p2m_segment.vstart;
            (*info).first_p2m_pfn = p2m_segment.pfn;
            (*info).nr_p2m_frames = p2m_segment.pages;
            (*info).flags = 0;
            (*info).store_evtchn = state.store_evtchn;
            (*info).store_mfn = setup.phys.p2m[state.xenstore_segment.pfn as usize];
            let console = state.consoles.first().unwrap();
            (*info).console.mfn = setup.phys.p2m[console.1.pfn as usize];
            (*info).console.evtchn = console.0;
            (*info).mod_start = state.initrd_segment.vstart;
            (*info).mod_len = state.initrd_segment.size;
            for (i, c) in cmdline.chars().enumerate() {
                (*info).cmdline[i] = c as c_char;
            }
            (*info).cmdline[MAX_GUEST_CMDLINE - 1] = 0;
            trace!("setup_start_info start_info={:?}", *info);
        }
        Ok(())
    }

    async fn setup_shared_info(
        &mut self,
        setup: &mut BootSetup,
        shared_info_frame: u64,
    ) -> Result<()> {
        let info = setup
            .phys
            .map_foreign_pages(shared_info_frame, X86_PAGE_SIZE)
            .await? as *mut SharedInfo;
        unsafe {
            let size = size_of::<SharedInfo>();
            let info_as_buff = slice::from_raw_parts_mut(info as *mut u8, size);
            info_as_buff.fill(0);
            for i in 0..32 {
                (*info).vcpu_info[i].evtchn_upcall_mask = 1;
            }
            trace!("setup_shared_info shared_info={:?}", *info);
        }
        Ok(())
    }

    async fn meminit(
        &mut self,
        setup: &mut BootSetup,
        total_pages: u64,
        _: &Option<DomainSegment>,
        _: &Option<DomainSegment>,
    ) -> Result<()> {
        setup.call.claim_pages(setup.domid, total_pages).await?;
        let mut vmemranges: Vec<VmemRange> = Vec::new();
        let stub = VmemRange {
            start: 0,
            end: total_pages << XEN_PAGE_SHIFT,
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

        if total != total_pages {
            return Err(Error::MemorySetupFailed("total pages mismatch"));
        }

        setup.total_pages = total;

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
                let extents = setup
                    .call
                    .populate_physmap(
                        setup.domid,
                        count,
                        SUPERPAGE_2MB_SHIFT as u32,
                        0,
                        &extents_init_slice[0usize..count as usize],
                    )
                    .await?;

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
                let result = setup
                    .call
                    .populate_physmap(setup.domid, allocsz, 0, 0, input_extent_starts)
                    .await?;

                if result.len() != allocsz as usize {
                    return Err(Error::PopulatePhysmapFailed(
                        allocsz as usize,
                        result.len(),
                        input_extent_starts.len(),
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

        setup.phys.load_p2m(p2m);
        setup.call.claim_pages(setup.domid, 0).await?;
        Ok(())
    }

    async fn bootlate(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()> {
        let p2m_segment = state
            .p2m_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("p2m_segment missing"))?;
        let page_table_segment = state
            .page_table_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("page_table_segment missing"))?;
        let pg_pfn = page_table_segment.pfn;
        setup.phys.unmap(pg_pfn)?;
        setup.phys.unmap(p2m_segment.pfn)?;

        let map = setup.call.get_memory_map(E820_MAX).await?;
        let mem_mb = setup.total_pages >> (20 - self.page_shift());
        let mem_kb = mem_mb * 1024;
        let e820 = self.e820_sanitize(map, mem_kb, 0)?;
        setup.call.set_memory_map(setup.domid, e820).await?;

        // setup
        //     .call
        //     .mmuext(setup.domid, MMUEXT_PIN_L4_TABLE, pg_mfn, 0)
        //     .await?;
        Ok(())
    }

    async fn vcpu(&mut self, setup: &mut BootSetup, state: &mut BootState) -> Result<()> {
        let page_table_segment = state
            .page_table_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("page_table_segment missing"))?;
        let pg_pfn = page_table_segment.pfn;
        let pg_mfn = setup.phys.p2m[pg_pfn as usize];
        let mut vcpu = VcpuGuestContext::default();
        vcpu.user_regs.rip = state.image_info.virt_entry;
        vcpu.user_regs.rsp =
            state.image_info.virt_base + (state.boot_stack_segment.pfn + 1) * self.page_size();
        vcpu.user_regs.rsi =
            state.image_info.virt_base + (state.start_info_segment.pfn) * self.page_size();
        vcpu.user_regs.rflags = 1 << 9;
        vcpu.debugreg[6] = 0xffff0ff0;
        vcpu.debugreg[7] = 0x00000400;
        vcpu.flags = VGCF_IN_KERNEL | VGCF_ONLINE;
        let cr3_pfn = pg_mfn;
        debug!("cr3: pfn {:#x} mfn {:#x}", page_table_segment.pfn, cr3_pfn);
        vcpu.ctrlreg[3] = cr3_pfn << 12;
        vcpu.user_regs.ds = 0x0;
        vcpu.user_regs.es = 0x0;
        vcpu.user_regs.fs = 0x0;
        vcpu.user_regs.gs = 0x0;
        vcpu.user_regs.ss = 0xe02b;
        vcpu.user_regs.cs = 0xe033;
        vcpu.kernel_ss = vcpu.user_regs.ss as u64;
        vcpu.kernel_sp = vcpu.user_regs.rsp;
        trace!("vcpu context: {:?}", vcpu);
        setup.call.set_vcpu_context(setup.domid, 0, &vcpu).await?;
        Ok(())
    }
}
