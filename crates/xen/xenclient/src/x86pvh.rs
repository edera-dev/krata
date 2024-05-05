use std::{
    mem::{size_of, MaybeUninit},
    os::raw::{c_char, c_void},
    ptr::addr_of_mut,
    slice,
};

use libc::munmap;
use nix::errno::Errno;
use xencall::sys::{
    ArchDomainConfig, CreateDomain, E820Entry, E820_RAM, E820_RESERVED,
    MEMFLAGS_POPULATE_ON_DEMAND, XEN_DOMCTL_CDF_HAP, XEN_DOMCTL_CDF_HVM_GUEST,
    XEN_DOMCTL_CDF_IOMMU, XEN_X86_EMU_LAPIC,
};

use crate::{
    boot::{BootDomain, BootSetupPlatform, DomainSegment},
    error::{Error, Result},
    sys::{
        GrantEntry, HVM_PARAM_ALTP2M, HVM_PARAM_BUFIOREQ_PFN, HVM_PARAM_CONSOLE_PFN,
        HVM_PARAM_IOREQ_PFN, HVM_PARAM_MONITOR_RING_PFN, HVM_PARAM_PAGING_RING_PFN,
        HVM_PARAM_SHARING_RING_PFN, HVM_PARAM_STORE_PFN, HVM_PARAM_TIMER_MODE, XEN_PAGE_SHIFT,
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

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct HvmSaveDescriptor {
    pub typecode: u16,
    pub instance: u16,
    pub length: u32,
}

#[repr(C)]
#[derive(Default, Copy, Clone, Debug)]
struct HvmSaveHeader {
    magic: u32,
    version: u32,
    changeset: u64,
    cpuid: u32,
    gtsc_khz: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct HvmCpu {
    pub fpu_regs: [u8; 512],
    pub rax: u64,
    pub rbx: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rbp: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub rsp: u64,
    pub r8: u64,
    pub r9: u64,
    pub r10: u64,
    pub r11: u64,
    pub r12: u64,
    pub r13: u64,
    pub r14: u64,
    pub r15: u64,
    pub rip: u64,
    pub rflags: u64,
    pub cr0: u64,
    pub cr2: u64,
    pub cr3: u64,
    pub cr4: u64,
    pub dr0: u64,
    pub dr1: u64,
    pub dr2: u64,
    pub dr3: u64,
    pub dr6: u64,
    pub dr7: u64,
    pub cs_sel: u32,
    pub ds_sel: u32,
    pub es_sel: u32,
    pub fs_sel: u32,
    pub gs_sel: u32,
    pub ss_sel: u32,
    pub tr_sel: u32,
    pub ldtr_sel: u32,
    pub cs_limit: u32,
    pub ds_limit: u32,
    pub es_limit: u32,
    pub fs_limit: u32,
    pub gs_limit: u32,
    pub ss_limit: u32,
    pub tr_limit: u32,
    pub ldtr_limit: u32,
    pub idtr_limit: u32,
    pub gdtr_limit: u32,
    pub cs_base: u64,
    pub ds_base: u64,
    pub es_base: u64,
    pub fs_base: u64,
    pub gs_base: u64,
    pub ss_base: u64,
    pub tr_base: u64,
    pub ldtr_base: u64,
    pub idtr_base: u64,
    pub gdtr_base: u64,
    pub cs_arbytes: u32,
    pub ds_arbytes: u32,
    pub es_arbytes: u32,
    pub fs_arbytes: u32,
    pub gs_arbytes: u32,
    pub ss_arbytes: u32,
    pub tr_arbytes: u32,
    pub ldtr_arbytes: u32,
    pub sysenter_cs: u64,
    pub sysenter_esp: u64,
    pub sysenter_eip: u64,
    pub shadow_gs: u64,
    pub msr_flags: u64,
    pub msr_lstar: u64,
    pub msr_star: u64,
    pub msr_cstar: u64,
    pub msr_syscall_mask: u64,
    pub msr_efer: u64,
    pub msr_tsc_aux: u64,
    pub tsc: u64,
    pub pending_event: u32,
    pub error_code: u32,
    pub flags: u32,
    pub pad0: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct HvmEnd {}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct BspCtx {
    header_d: HvmSaveDescriptor,
    header: HvmSaveHeader,
    cpu_d: HvmSaveDescriptor,
    cpu: HvmCpu,
    end_d: HvmSaveDescriptor,
    end: HvmEnd,
}

#[derive(Debug)]
struct VmemRange {
    start: u64,
    end: u64,
    _flags: u32,
    _nid: u32,
}

#[derive(Default, Clone)]
pub struct X86PvhPlatform {
    start_info_segment: Option<DomainSegment>,
    lowmem_end: u64,
    highmem_end: u64,
    mmio_start: u64,
}

const X86_CR0_PE: u64 = 0x01;
const X86_CR0_ET: u64 = 0x10;

impl X86PvhPlatform {
    pub fn new() -> Self {
        Self {
            ..Default::default()
        }
    }

    pub fn construct_memmap(&self) -> Result<Vec<E820Entry>> {
        let mut entries = Vec::new();

        let highmem_size = if self.highmem_end > 0 {
            self.highmem_end - (1u64 << 32)
        } else {
            0
        };
        let lowmem_start = 0u64;
        entries.push(E820Entry {
            addr: lowmem_start,
            size: self.lowmem_end - lowmem_start,
            typ: E820_RAM,
        });

        entries.push(E820Entry {
            addr: (X86_HVM_END_SPECIAL_REGION - X86_HVM_NR_SPECIAL_PAGES) << XEN_PAGE_SHIFT,
            size: X86_HVM_NR_SPECIAL_PAGES << XEN_PAGE_SHIFT,
            typ: E820_RESERVED,
        });

        if highmem_size > 0 {
            entries.push(E820Entry {
                addr: 1u64 << 32,
                size: highmem_size,
                typ: E820_RAM,
            });
        }

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
    fn create_domain(&self) -> CreateDomain {
        CreateDomain {
            flags: XEN_DOMCTL_CDF_HVM_GUEST | XEN_DOMCTL_CDF_HAP | XEN_DOMCTL_CDF_IOMMU,
            arch_domain_config: ArchDomainConfig {
                emulation_flags: XEN_X86_EMU_LAPIC,
                ..Default::default()
            },
            ..Default::default()
        }
    }

    fn page_size(&self) -> u64 {
        X86_PAGE_SIZE
    }

    fn page_shift(&self) -> u64 {
        X86_PAGE_SHIFT
    }

    fn needs_early_kernel(&self) -> bool {
        false
    }

    async fn initialize_early(&mut self, domain: &mut BootDomain) -> Result<()> {
        let mut memory_start =
            (X86_HVM_END_SPECIAL_REGION - X86_HVM_NR_SPECIAL_PAGES) << self.page_shift();
        memory_start = memory_start.min(LAPIC_BASE_ADDRESS);
        memory_start = memory_start.min(ACPI_INFO_PHYSICAL_ADDRESS);
        let mmio_size = (4 * 1024 * 1024 * 1024) - memory_start;
        let mut lowmem_end = domain.total_pages << self.page_shift();
        let mut highmem_end = 0u64;
        let mmio_start = (1u64 << 32) - mmio_size;

        if lowmem_end > mmio_start {
            highmem_end = (1 << 32) + (lowmem_end - mmio_start);
            lowmem_end = mmio_start;
        }
        self.lowmem_end = lowmem_end;
        self.highmem_end = highmem_end;
        self.mmio_start = mmio_start;

        domain
            .call
            .set_hvm_param(domain.domid, HVM_PARAM_TIMER_MODE, 1)
            .await?;
        domain
            .call
            .set_hvm_param(domain.domid, HVM_PARAM_ALTP2M, 0)
            .await?;
        domain
            .call
            .set_paging_mempool_size(domain.domid, 1024 << 12)
            .await?;
        let memmap = self.construct_memmap()?;
        domain
            .call
            .set_memory_map(domain.domid, memmap.clone())
            .await?;
        Ok(())
    }

    async fn initialize_memory(&mut self, domain: &mut BootDomain) -> Result<()> {
        domain
            .call
            .claim_pages(domain.domid, domain.total_pages)
            .await?;
        let memflags = if domain.target_pages > domain.total_pages {
            MEMFLAGS_POPULATE_ON_DEMAND
        } else {
            0
        };

        let mut vmemranges: Vec<VmemRange> = Vec::new();
        vmemranges.push(VmemRange {
            start: 0,
            end: self.lowmem_end,
            _flags: 0,
            _nid: 0,
        });

        if self.highmem_end > (1u64 << 32) {
            vmemranges.push(VmemRange {
                start: 1u64 << 32,
                end: self.highmem_end,
                _flags: 0,
                _nid: 0,
            });
        }

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
            let end_pages = range.end >> self.page_shift();
            let mut cur_pages = range.start >> self.page_shift();

            while end_pages > cur_pages {
                let count = end_pages - cur_pages;
                if count != 0 {
                    let mut extents = vec![0u64; count as usize];

                    for i in 0..count {
                        extents[i as usize] = cur_pages + i;
                    }

                    let _ = domain
                        .call
                        .populate_physmap(domain.domid, count, 0_u32, memflags, &extents)
                        .await?;
                    cur_pages += count;
                }
            }
        }

        domain.call.claim_pages(domain.domid, 0).await?;

        Ok(())
    }

    async fn alloc_p2m_segment(&mut self, _: &mut BootDomain) -> Result<Option<DomainSegment>> {
        Ok(None)
    }

    async fn alloc_page_tables(&mut self, _: &mut BootDomain) -> Result<Option<DomainSegment>> {
        Ok(None)
    }

    async fn setup_page_tables(&mut self, _: &mut BootDomain) -> Result<()> {
        Ok(())
    }

    async fn setup_hypercall_page(&mut self, _: &mut BootDomain) -> Result<()> {
        Ok(())
    }

    async fn alloc_magic_pages(&mut self, domain: &mut BootDomain) -> Result<()> {
        let memmap = self.construct_memmap()?;
        let mut special_array = vec![0u64; X86_HVM_NR_SPECIAL_PAGES as usize];
        for i in 0..X86_HVM_NR_SPECIAL_PAGES {
            special_array[i as usize] = special_pfn(i as u32);
        }
        domain
            .call
            .populate_physmap(
                domain.domid,
                special_array.len() as u64,
                0,
                0,
                &special_array,
            )
            .await?;
        domain
            .phys
            .clear_pages(special_pfn(0), special_array.len() as u64)
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_STORE_PFN,
                special_pfn(SPECIALPAGE_XENSTORE),
            )
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_BUFIOREQ_PFN,
                special_pfn(SPECIALPAGE_BUFIOREQ),
            )
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_IOREQ_PFN,
                special_pfn(SPECIALPAGE_IOREQ),
            )
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_CONSOLE_PFN,
                special_pfn(SPECIALPAGE_CONSOLE),
            )
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_PAGING_RING_PFN,
                special_pfn(SPECIALPAGE_PAGING),
            )
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_MONITOR_RING_PFN,
                special_pfn(SPECIALPAGE_ACCESS),
            )
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_SHARING_RING_PFN,
                special_pfn(SPECIALPAGE_SHARING),
            )
            .await?;

        let mut start_info_size = size_of::<HvmStartInfo>();

        start_info_size += BootDomain::round_up("".len() as u64 + 3, 3) as usize;
        start_info_size += size_of::<E820Entry>() * memmap.len();

        self.start_info_segment = Some(domain.alloc_segment(0, start_info_size as u64).await?);
        domain.consoles.push((0, special_pfn(SPECIALPAGE_CONSOLE)));
        domain.store_mfn = special_pfn(SPECIALPAGE_XENSTORE);

        Ok(())
    }

    async fn setup_shared_info(&mut self, _: &mut BootDomain, _: u64) -> Result<()> {
        Ok(())
    }

    async fn setup_start_info(&mut self, _: &mut BootDomain, _: &str, _: u64) -> Result<()> {
        Ok(())
    }

    async fn bootlate(&mut self, _: &mut BootDomain) -> Result<()> {
        Ok(())
    }

    async fn vcpu(&mut self, domain: &mut BootDomain) -> Result<()> {
        let size = domain.call.get_hvm_context(domain.domid, None).await?;
        let mut full_context = vec![0u8; size as usize];
        domain
            .call
            .get_hvm_context(domain.domid, Some(&mut full_context))
            .await?;
        let mut ctx: BspCtx = unsafe { MaybeUninit::zeroed().assume_init() };
        unsafe {
            std::ptr::copy(
                full_context.as_ptr(),
                addr_of_mut!(ctx) as *mut u8,
                size_of::<HvmSaveDescriptor>() + size_of::<HvmSaveHeader>(),
            )
        };
        ctx.cpu_d.instance = 0;
        ctx.cpu.cs_base = 0;
        ctx.cpu.ds_base = 0;
        ctx.cpu.es_base = 0;
        ctx.cpu.ss_base = 0;
        ctx.cpu.tr_base = 0;
        ctx.cpu.cs_limit = !0;
        ctx.cpu.ds_limit = !0;
        ctx.cpu.es_limit = !0;
        ctx.cpu.ss_limit = !0;
        ctx.cpu.tr_limit = 0x67;
        ctx.cpu.cs_arbytes = 0xc9b;
        ctx.cpu.ds_arbytes = 0xc93;
        ctx.cpu.es_arbytes = 0xc93;
        ctx.cpu.ss_arbytes = 0xc93;
        ctx.cpu.tr_arbytes = 0x8b;
        ctx.cpu.cr0 = X86_CR0_PE | X86_CR0_ET;
        ctx.cpu.rip = domain.image_info.virt_entry;
        ctx.cpu.dr6 = 0xffff0ff0;
        ctx.cpu.dr7 = 0x00000400;
        let addr = addr_of_mut!(ctx) as *mut u8;
        let slice = unsafe { std::slice::from_raw_parts_mut(addr, size_of::<BspCtx>()) };
        domain.call.set_hvm_context(domain.domid, slice).await?;
        Ok(())
    }

    async fn gnttab_seed(&mut self, domain: &mut BootDomain) -> Result<()> {
        let console_gfn = domain.consoles.first().map(|x| x.1).unwrap_or(0) as usize;
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
        entries[1].frame = domain.store_mfn as u32;
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
const SPECIALPAGE_ACCESS: u32 = 1;
const SPECIALPAGE_SHARING: u32 = 2;
const SPECIALPAGE_BUFIOREQ: u32 = 3;
const SPECIALPAGE_XENSTORE: u32 = 4;
const SPECIALPAGE_IOREQ: u32 = 5;
const _SPECIALPAGE_IDENT_PT: u32 = 6;
const SPECIALPAGE_CONSOLE: u32 = 7;
const LAPIC_BASE_ADDRESS: u64 = 0xfee00000;
const ACPI_INFO_PHYSICAL_ADDRESS: u64 = 0xFC000000;
