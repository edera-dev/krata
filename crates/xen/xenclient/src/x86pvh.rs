use std::{
    mem::size_of,
    os::raw::{c_char, c_void},
    slice,
};

use libc::munmap;
use log::trace;
use nix::errno::Errno;
use xencall::sys::{
    x8664VcpuGuestContext, E820Entry, E820_RAM, MEMFLAGS_POPULATE_ON_DEMAND
};

use crate::{
    boot::{BootDomain, BootSetupPlatform, DomainSegment},
    error::{Error, Result},
    sys::{
        GrantEntry, HVM_PARAM_BUFIOREQ_PFN, HVM_PARAM_CONSOLE_PFN, HVM_PARAM_IOREQ_PFN, HVM_PARAM_MONITOR_RING_PFN, HVM_PARAM_PAGING_RING_PFN, HVM_PARAM_SHARING_RING_PFN, HVM_PARAM_STORE_PFN, SUPERPAGE_1GB_NR_PFNS, SUPERPAGE_1GB_SHIFT, SUPERPAGE_2MB_NR_PFNS, SUPERPAGE_2MB_SHIFT, SUPERPAGE_BATCH_SIZE, VGCF_IN_KERNEL, VGCF_ONLINE, XEN_PAGE_SHIFT
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
#[derive(Default, Copy, Clone, Debug)]
pub struct HvmStartInfo {
    pub magic: u32,
    pub version: u32,
    pub flags: u32,
    pub nr_modules: u32,
    pub modlist_paddr: u64,
    pub cmdline_paddr: u64,
    pub rsdp_paddr: u64,
    pub memmap_paddr: u64,
    pub memmap_entries: u32,
    pub reserved: u32,
}

#[repr(C)]
#[derive(Default, Copy, Clone, Debug)]
pub struct HvmModlistEntry {
    pub paddr: u64,
    pub size: u64,
    pub cmdline_paddr: u64,
    pub reserved: u64,
}

#[repr(C)]
#[derive(Default, Copy, Clone, Debug)]
pub struct HvmMemmapTableEntry {
    pub addr: u64,
    pub size: u64,
    pub typ: u32,
    pub reserved: u32,
}

const HVMLOADER_MODULE_MAX_COUNT: u32 = 2;

#[derive(Debug)]
struct VmemRange {
    start: u64,
    end: u64,
    _flags: u32,
    nid: u32,
}

#[derive(Default)]
pub struct X86PvhPlatform {
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

    pub fn construct_memmap(&self, mem_size_bytes: u64) -> Result<Vec<E820Entry>> {
        let entries = vec![
            E820Entry {
                addr: 0,
                size: mem_size_bytes,
                typ: E820_RAM
            },
            E820Entry {
                addr: (X86_HVM_END_SPECIAL_REGION - X86_HVM_NR_SPECIAL_PAGES) << XEN_PAGE_SHIFT,
                size: X86_HVM_NR_SPECIAL_PAGES << XEN_PAGE_SHIFT,
                typ: E820_RAM
            },
        ];
        Ok(entries)
    }

    const _PAGE_PRESENT: u64 = 0x001;
    const _PAGE_RW: u64 = 0x002;
    const _PAGE_USER: u64 = 0x004;
    const _PAGE_ACCESSED: u64 = 0x020;
    const _PAGE_DIRTY: u64 = 0x040;
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
        let memmap = self.construct_memmap(domain.total_pages << XEN_PAGE_SHIFT)?;
        domain.call.set_memory_map(domain.domid, memmap.clone()).await?;

        let mut special_array = vec![0u64; X86_HVM_NR_SPECIAL_PAGES as usize];
        for i in 0..X86_HVM_NR_SPECIAL_PAGES {
            special_array[i as usize] = special_pfn(i as u32);
        }
        let _pages = domain.call.populate_physmap(domain.domid, X86_HVM_NR_SPECIAL_PAGES, 0, 0, &special_array).await?;
        domain.phys.clear_pages(special_pfn(0), X86_HVM_NR_SPECIAL_PAGES).await?;
        domain.call.set_hvm_param(domain.domid, HVM_PARAM_STORE_PFN, special_pfn(SPECIALPAGE_XENSTORE)).await?;
        domain.call.set_hvm_param(domain.domid, HVM_PARAM_BUFIOREQ_PFN, special_pfn(SPECIALPAGE_BUFIOREQ)).await?;
        domain.call.set_hvm_param(domain.domid, HVM_PARAM_IOREQ_PFN, special_pfn(SPECIALPAGE_IOREQ)).await?;
        domain.call.set_hvm_param(domain.domid, HVM_PARAM_CONSOLE_PFN, special_pfn(SPECIALPAGE_CONSOLE)).await?;
        domain.call.set_hvm_param(domain.domid, HVM_PARAM_PAGING_RING_PFN, special_pfn(SPECIALPAGE_PAGING)).await?;
        domain.call.set_hvm_param(domain.domid, HVM_PARAM_MONITOR_RING_PFN, special_pfn(SPECIALPAGE_ACCESS)).await?;
        domain.call.set_hvm_param(domain.domid, HVM_PARAM_SHARING_RING_PFN, special_pfn(SPECIALPAGE_SHARING)).await?;

        let mut start_info_size = size_of::<HvmStartInfo>();

        start_info_size += BootDomain::round_up("".len() as u64 + 3, 3) as usize;
        start_info_size += size_of::<E820Entry>() * memmap.len();

        self.start_info_segment = Some(domain.alloc_segment(0, start_info_size as u64).await?);
        domain.consoles.push((0, special_pfn(SPECIALPAGE_CONSOLE)));
        domain.xenstore_mfn = special_pfn(SPECIALPAGE_XENSTORE);

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
        let pg_pfn = 0;
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

fn special_pfn(x: u32) -> u64 {
    (X86_HVM_END_SPECIAL_REGION - X86_HVM_NR_SPECIAL_PAGES) + (x as u64)
}

const X86_HVM_NR_SPECIAL_PAGES: u64 = 8;
const X86_HVM_END_SPECIAL_REGION: u64 = 0xff000;

const SPECIALPAGE_PAGING: u32 = 0;
const SPECIALPAGE_ACCESS: u32  = 1;
const SPECIALPAGE_SHARING: u32 =  2;
const SPECIALPAGE_BUFIOREQ: u32 = 3;
const SPECIALPAGE_XENSTORE: u32 = 4;
const SPECIALPAGE_IOREQ : u32 =   5;
const SPECIALPAGE_IDENT_PT: u32 = 6;
const SPECIALPAGE_CONSOLE: u32 =  7;
