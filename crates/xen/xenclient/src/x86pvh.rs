use std::{
    mem::size_of,
    os::raw::{c_char, c_void},
    slice,
};

use libc::munmap;
use log::{debug, trace};
use nix::errno::Errno;
use slice_copy::copy;
use xencall::sys::{
    x8664VcpuGuestContext, E820Entry, E820_MAX, E820_RAM, E820_UNUSABLE, MEMFLAGS_POPULATE_ON_DEMAND
};

use crate::{
    boot::{BootDomain, BootSetupPlatform, DomainSegment},
    error::{Error, Result},
    sys::{
        GrantEntry, SUPERPAGE_1GB_NR_PFNS, SUPERPAGE_1GB_SHIFT, SUPERPAGE_2MB_NR_PFNS, SUPERPAGE_2MB_SHIFT, SUPERPAGE_BATCH_SIZE, VGCF_IN_KERNEL, VGCF_ONLINE, XEN_PAGE_SHIFT
    },
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

#[derive(Debug)]
struct VmemRange {
    start: u64,
    end: u64,
    _flags: u32,
    nid: u32,
}

#[derive(Default)]
pub struct X86PvhPlatform {
    table: PageTable,
    p2m_segment: Option<DomainSegment>,
    page_table_segment: Option<DomainSegment>,
    start_info_segment: Option<DomainSegment>,
    boot_stack_segment: Option<DomainSegment>,
    xenstore_segment: Option<DomainSegment>,
}

impl X86PvhPlatform {
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    const PAGE_PRESENT: u64 = 0x001;
    const PAGE_RW: u64 = 0x002;
    const PAGE_USER: u64 = 0x004;
    const PAGE_ACCESSED: u64 = 0x020;
    const PAGE_DIRTY: u64 = 0x040;
    fn get_pg_prot(&mut self, l: usize, pfn: u64) -> u64 {
        let prot = [
            X86PvhPlatform::PAGE_PRESENT | X86PvhPlatform::PAGE_RW | X86PvhPlatform::PAGE_ACCESSED,
            X86PvhPlatform::PAGE_PRESENT
                | X86PvhPlatform::PAGE_RW
                | X86PvhPlatform::PAGE_ACCESSED
                | X86PvhPlatform::PAGE_DIRTY
                | X86PvhPlatform::PAGE_USER,
            X86PvhPlatform::PAGE_PRESENT
                | X86PvhPlatform::PAGE_RW
                | X86PvhPlatform::PAGE_ACCESSED
                | X86PvhPlatform::PAGE_DIRTY
                | X86PvhPlatform::PAGE_USER,
            X86PvhPlatform::PAGE_PRESENT
                | X86PvhPlatform::PAGE_RW
                | X86PvhPlatform::PAGE_ACCESSED
                | X86PvhPlatform::PAGE_DIRTY
                | X86PvhPlatform::PAGE_USER,
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
                return prot & !X86PvhPlatform::PAGE_RW;
            }
        }
        prot
    }

    fn count_page_tables(
        &mut self,
        domain: &mut BootDomain,
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
        if pfn_end >= domain.phys.p2m_size() {
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
            map.levels[l].pfn = domain.pfn_alloc_end + map.area.pgtables as u64;
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
            let mask = BootDomain::bits_to_mask(bits);
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
impl BootSetupPlatform for X86PvhPlatform {
    fn page_size(&self) -> u64 {
        X86_PAGE_SIZE
    }

    fn page_shift(&self) -> u64 {
        X86_PAGE_SHIFT
    }

    fn needs_early_kernel(&self) -> bool {
        false
    }

    async fn initialize_memory(&self, domain: &mut BootDomain) -> Result<()> {
        let memflags = if domain.target_pages > domain.total_pages {
            MEMFLAGS_POPULATE_ON_DEMAND
        } else {
            0
        };

        let mut vmemranges: Vec<VmemRange> = Vec::new();
        let stub = VmemRange {
            start: 0,
            end: domain.total_pages << self.page_shift(),
            _flags: 0,
            nid: 0,
        };
        vmemranges.push(stub);

        let mut p2m_size: u64 = 0;
        let mut total: u64 = 0;
        for range in &vmemranges {
            total += (range.end - range.start) >> XEN_PAGE_SHIFT;
            p2m_size = p2m_size.max(range.end >> XEN_PAGE_SHIFT);
        }

        if total != domain.total_pages {
            return Err(Error::MemorySetupFailed("total pages mismatch"));
        }

        for range in &vmemranges {
            let memflags = memflags;

            let end_pages = range.end >> self.page_shift();
            let mut cur_pages = range.start >> self.page_shift();

            while end_pages > cur_pages {
                let count = end_pages - cur_pages;
                if count != 0 {
                    let mut extents = vec![0u64; count as usize];

                    for i in 0..count {
                        extents[i as usize] = cur_pages + i;
                    }

                    let _ = domain.call.populate_physmap(domain.domid, count, 0 as u32, memflags, &extents).await?;
                    cur_pages += count as u64;
                }
            }
        }

        Ok(())
    }

    async fn alloc_p2m_segment(
        &mut self,
        _: &mut BootDomain,
    ) -> Result<Option<DomainSegment>> {
        Ok(None)
    }

    async fn alloc_page_tables(
        &mut self,
        _: &mut BootDomain,
    ) -> Result<Option<DomainSegment>> {
        Ok(None)
    }

    async fn setup_page_tables(&mut self, _: &mut BootDomain) -> Result<()> {
        Ok(())
    }

    async fn setup_hypercall_page(&mut self, _: &mut BootDomain) -> Result<()> {
        Ok(())
    }

    async fn alloc_magic_pages(&mut self, domain: &mut BootDomain) -> Result<()> {
        let mut special_array = vec![0u64; X86_HVM_NR_SPECIAL_PAGES as usize];
        for i in 0..X86_HVM_NR_SPECIAL_PAGES {
            special_array[i as usize] = special_pfn(i);
        }
        let pages = domain.call.populate_physmap(domain.domid, 8, 0, 0, &special_array).await?;

        

        Ok(())
    }

    async fn setup_shared_info(
        &mut self,
        _: &mut BootDomain,
        _: u64,
    ) -> Result<()> {
        Ok(())
    }

    async fn setup_start_info(
        &mut self,
        _: &mut BootDomain,
        _: &str,
        _: u64,
    ) -> Result<()> {
       Ok(())
    }

    async fn bootlate(&mut self, domain: &mut BootDomain) -> Result<()> {
        let map = domain.call.get_memory_map(E820_MAX).await?;
        let mem_mb = domain.total_pages >> (20 - self.page_shift());
        let mem_kb = mem_mb * 1024;
        let e820 = self.e820_sanitize(map, mem_kb, 0)?;
        domain.call.set_memory_map(domain.domid, e820).await?;
        Ok(())
    }

    async fn vcpu(&mut self, domain: &mut BootDomain) -> Result<()> {
        let boot_stack_segment = self
            .boot_stack_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("boot_stack_segment missing"))?;
        let start_info_segment = self
            .start_info_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("start_info_segment missing"))?;
        let pg_pfn = page_table_segment.pfn;
        let pg_mfn = domain.phys.p2m[pg_pfn as usize];
        let mut vcpu = x8664VcpuGuestContext::default();
        vcpu.user_regs.rip = domain.image_info.virt_entry;
        vcpu.user_regs.rsp =
            domain.image_info.virt_base + (boot_stack_segment.pfn + 1) * self.page_size();
        vcpu.user_regs.rsi =
            domain.image_info.virt_base + (start_info_segment.pfn) * self.page_size();
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
        domain.call.set_vcpu_context(domain.domid, 0, xencall::sys::VcpuGuestContextAny { value: vcpu }).await?;
        Ok(())
    }

    async fn gnttab_seed(&mut self, domain: &mut BootDomain) -> Result<()> {
        let xenstore_segment = self
            .xenstore_segment
            .as_ref()
            .ok_or(Error::MemorySetupFailed("xenstore_segment missing"))?;

        let console_gfn = domain.consoles.first().map(|x| x.1).unwrap_or(0) as usize;
        let xenstore_gfn = domain.phys.p2m[xenstore_segment.pfn as usize];
        let addr = domain
            .call
            .mmap(0, 1 << XEN_PAGE_SHIFT)
            .await
            .ok_or(Error::MmapFailed)?;
        domain
            .call
            .map_resource(domain.domid, 1, 0, 0, 1, addr)
            .await?;
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
                return Err(Error::UnmapFailed(Errno::from_raw(result)));
            }
        }
        Ok(())
    }
}

fn special_pfn(x: u64) -> u64 {
    X86_HVM_END_SPECIAL_REGION - X86_HVM_NR_SPECIAL_PAGES + x
}

const X86_HVM_NR_SPECIAL_PAGES: u64 = 8;
const X86_HVM_END_SPECIAL_REGION: u64 = 0xff000;
