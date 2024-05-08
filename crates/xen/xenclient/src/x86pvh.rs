use std::{
    mem::{size_of, MaybeUninit},
    ptr::{addr_of, addr_of_mut},
    slice,
};

use libc::{c_void, malloc, munmap};
use nix::errno::Errno;
use xencall::sys::{
    ArchDomainConfig, CreateDomain, E820Entry, E820_ACPI, E820_RAM, E820_RESERVED,
    MEMFLAGS_POPULATE_ON_DEMAND, XEN_DOMCTL_CDF_HAP, XEN_DOMCTL_CDF_HVM_GUEST,
    XEN_DOMCTL_CDF_IOMMU, XEN_X86_EMU_LAPIC,
};

use crate::{
    boot::{BootDomain, BootSetupPlatform, DomainSegment},
    error::{Error, Result},
    sys::{
        GrantEntry, HVM_PARAM_ALTP2M, HVM_PARAM_BUFIOREQ_PFN, HVM_PARAM_CONSOLE_EVTCHN,
        HVM_PARAM_CONSOLE_PFN, HVM_PARAM_IDENT_PT, HVM_PARAM_IOREQ_PFN, HVM_PARAM_MONITOR_RING_PFN,
        HVM_PARAM_PAGING_RING_PFN, HVM_PARAM_SHARING_RING_PFN, HVM_PARAM_STORE_EVTCHN,
        HVM_PARAM_STORE_PFN, HVM_PARAM_TIMER_MODE, XEN_HVM_START_MAGIC_VALUE, XEN_PAGE_SHIFT,
    },
    x86acpi::{acpi_build_tables, acpi_config, acpi_ctxt, acpi_mem_ops, dsdt_pvh, hvm_info_table},
};

const X86_PAGE_SHIFT: u64 = 12;
const X86_PAGE_SIZE: u64 = 1 << X86_PAGE_SHIFT;

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

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct HvmMtrr {
    msr_pat_cr: u64,
    msr_mtrr_var: [u64; 16],
    msr_mtrr_fixed: [u64; 11],
    msr_mtrr_cap: u64,
    msr_mtrr_def_type: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
struct MtrrCtx {
    header_d: HvmSaveDescriptor,
    header: HvmSaveHeader,
    mtrr_d: HvmSaveDescriptor,
    mtrr: HvmMtrr,
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

#[derive(Debug, Clone)]
struct AcpiModule {
    data: u64,
    length: u32,
    guest_addr: u64,
}

#[derive(Default, Clone)]
pub struct X86PvhPlatform {
    start_info_segment: Option<DomainSegment>,
    lowmem_end: u64,
    highmem_end: u64,
    mmio_start: u64,
    acpi_modules: Vec<AcpiModule>,
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

        for module in &self.acpi_modules {
            entries.push(E820Entry {
                addr: module.guest_addr & !(self.page_size() - 1),
                size: module.length as u64 + (module.guest_addr & (self.page_size() - 1)),
                typ: E820_ACPI,
            });
        }

        if highmem_size > 0 {
            entries.push(E820Entry {
                addr: 1u64 << 32,
                size: highmem_size,
                typ: E820_RAM,
            });
        }

        Ok(entries)
    }

    unsafe fn get_save_record<T: Sized>(ctx: &mut [u8], typ: u16, instance: u16) -> *mut T {
        let mut ptr = ctx.as_mut_ptr();
        loop {
            let sd = ptr as *mut HvmSaveDescriptor;

            if (*sd).typecode == 0 {
                break;
            }

            if (*sd).typecode == typ && (*sd).instance == instance {
                return ptr.add(size_of::<HvmSaveDescriptor>()) as *mut T;
            }
            ptr = ptr
                .add(size_of::<HvmSaveDescriptor>())
                .add((*sd).length as usize);
        }
        std::ptr::null_mut()
    }

    const PAGE_PRESENT: u32 = 0x001;
    const PAGE_RW: u32 = 0x002;
    const PAGE_USER: u32 = 0x004;
    const PAGE_ACCESSED: u32 = 0x020;
    const PAGE_DIRTY: u32 = 0x040;
    const PAGE_PSE: u32 = 0x080;
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
        {
            let mut config: acpi_config = unsafe { MaybeUninit::zeroed().assume_init() };
            let mut hvminfo: Vec<hvm_info_table> =
                vec![unsafe { MaybeUninit::zeroed().assume_init() }; 1];
            config.hvminfo = hvminfo.as_ptr();
            let h = hvminfo.get_mut(0).unwrap();
            h.nr_vcpus = domain.max_vcpus;
            for i in 0..domain.max_vcpus {
                h.vcpu_online[(i / 8) as usize] |= 1 << (i & 7);
            }
            config.lapic_base_address = LAPIC_BASE_ADDRESS as u32;
            config.lapic_id = Some(acpi_lapic_id);
            config.acpi_revision = 5;
            unsafe {
                config.dsdt_15cpu = addr_of!(dsdt_pvh) as *mut u8;
                config.dsdt_15cpu_len = dsdt_pvh.len() as u32;
                config.dsdt_anycpu = addr_of!(dsdt_pvh) as *mut u8;
                config.dsdt_anycpu_len = dsdt_pvh.len() as u32;
            };
            unsafe {
                config.rsdp = malloc(self.page_size() as usize) as u64;
                config.infop = malloc(self.page_size() as usize) as u64;
                let buf = malloc((8 * self.page_size()) as usize);
                let mut ctx = AcpiBuildContext {
                    page_size: self.page_size(),
                    page_shift: self.page_shift(),
                    buf: buf as *mut u8,
                    guest_start: ACPI_INFO_PHYSICAL_ADDRESS + self.page_size(),
                    guest_curr: ACPI_INFO_PHYSICAL_ADDRESS + self.page_size(),
                    guest_end: ACPI_INFO_PHYSICAL_ADDRESS
                        + self.page_size()
                        + (8 * self.page_size()),
                };
                let mut ctxt = acpi_ctxt {
                    ptr: addr_of_mut!(ctx) as *mut c_void,
                    mem_ops: acpi_mem_ops {
                        alloc: Some(acpi_mem_alloc),
                        free: Some(acpi_mem_free),
                        v2p: Some(acpi_v2p),
                    },
                };
                if acpi_build_tables(addr_of_mut!(ctxt), addr_of_mut!(config)) > 0 {
                    return Err(Error::GenericError("acpi_build_tables failed".to_string()));
                }

                let acpi_pages_num = (align_it(ctx.guest_curr, self.page_size()) - ctx.guest_start)
                    >> self.page_shift();
                self.acpi_modules.push(AcpiModule {
                    data: config.rsdp,
                    length: 64,
                    guest_addr: ACPI_INFO_PHYSICAL_ADDRESS
                        + (1 + acpi_pages_num) * self.page_size(),
                });

                self.acpi_modules.push(AcpiModule {
                    data: config.infop,
                    length: 4096,
                    guest_addr: ACPI_INFO_PHYSICAL_ADDRESS,
                });

                self.acpi_modules.push(AcpiModule {
                    data: ctx.buf as u64,
                    length: (acpi_pages_num << self.page_shift()) as u32,
                    guest_addr: ACPI_INFO_PHYSICAL_ADDRESS + self.page_size(),
                });
            }
        }

        {
            for module in &self.acpi_modules {
                let num_pages = ((module.length as u64
                    + (module.guest_addr & !(!(self.page_size() - 1))))
                    + (self.page_size() - 1))
                    >> self.page_shift();
                let base = module.guest_addr >> self.page_shift();
                for i in 0..num_pages {
                    let e = base + i;
                    if domain
                        .call
                        .populate_physmap(domain.domid, 1, 0, 0, &[e])
                        .await
                        .unwrap_or(Vec::new())
                        .len()
                        == 1
                    {
                        continue;
                    }

                    let idx = self.lowmem_end;
                    self.lowmem_end -= 1;
                    domain.call.add_to_physmap(domain.domid, 2, idx, e).await?;
                }

                let ptr = domain
                    .phys
                    .map_foreign_pages(base, num_pages << self.page_shift())
                    .await? as *mut u8;
                let dst = unsafe { std::slice::from_raw_parts_mut(ptr, module.length as usize) };
                let src = unsafe {
                    std::slice::from_raw_parts(module.data as *mut u8, module.length as usize)
                };
                slice_copy::copy(dst, src);
            }
        }

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
        start_info_size += domain.cmdline.len() + 1;
        start_info_size += size_of::<HvmMemmapTableEntry>() * memmap.len();
        self.start_info_segment = Some(domain.alloc_segment(0, start_info_size as u64).await?);

        let pt = domain
            .phys
            .map_foreign_pages(special_pfn(SPECIALPAGE_IDENT_PT), self.page_size())
            .await? as *mut u32;
        for i in 0..(self.page_size() / size_of::<u32>() as u64) {
            unsafe {
                *(pt.offset(i as isize)) = ((i as u32) << 22)
                    | X86PvhPlatform::PAGE_PRESENT
                    | X86PvhPlatform::PAGE_RW
                    | X86PvhPlatform::PAGE_USER
                    | X86PvhPlatform::PAGE_ACCESSED
                    | X86PvhPlatform::PAGE_DIRTY
                    | X86PvhPlatform::PAGE_PSE;
            }
        }
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_IDENT_PT,
                special_pfn(SPECIALPAGE_IDENT_PT),
            )
            .await?;

        let evtchn = domain.call.evtchn_alloc_unbound(domain.domid, 0).await?;
        domain
            .consoles
            .push((evtchn, special_pfn(SPECIALPAGE_CONSOLE)));
        domain.store_mfn = special_pfn(SPECIALPAGE_XENSTORE);

        Ok(())
    }

    async fn setup_shared_info(&mut self, _: &mut BootDomain, _: u64) -> Result<()> {
        Ok(())
    }

    async fn setup_start_info(&mut self, domain: &mut BootDomain, _: &str, _: u64) -> Result<()> {
        let memmap = self.construct_memmap()?;
        let start_info_segment = self
            .start_info_segment
            .as_ref()
            .ok_or_else(|| Error::GenericError("start_info_segment missing".to_string()))?;
        let ptr = domain
            .phys
            .pfn_to_ptr(start_info_segment.pfn, start_info_segment.pages)
            .await?;
        let byte_slice = unsafe {
            slice::from_raw_parts_mut(
                ptr as *mut u8,
                (self.page_size() * start_info_segment.pages) as usize,
            )
        };
        byte_slice.fill(0);
        let info = ptr as *mut HvmStartInfo;
        unsafe {
            (*info).magic = XEN_HVM_START_MAGIC_VALUE;
            (*info).version = 1;
            (*info).cmdline_paddr =
                (start_info_segment.pfn << self.page_shift()) + size_of::<HvmStartInfo>() as u64;
            (*info).memmap_paddr = (start_info_segment.pfn << self.page_shift())
                + size_of::<HvmStartInfo>() as u64
                + domain.cmdline.len() as u64
                + 1;
            (*info).memmap_entries = memmap.len() as u32;
            (*info).rsdp_paddr = self.acpi_modules[0].guest_addr;
        };
        let cmdline_ptr = (ptr + size_of::<HvmStartInfo>() as u64) as *mut u8;
        for (i, c) in domain.cmdline.chars().enumerate() {
            unsafe { *cmdline_ptr.add(i) = c as u8 };
        }
        let entries = (ptr + size_of::<HvmStartInfo>() as u64 + domain.cmdline.len() as u64 + 1)
            as *mut HvmMemmapTableEntry;
        let entries = unsafe { std::slice::from_raw_parts_mut(entries, memmap.len()) };
        for (i, e820) in memmap.iter().enumerate() {
            let entry = &mut entries[i];
            entry.addr = e820.addr;
            entry.size = e820.size;
            entry.typ = e820.typ;
            entry.reserved = 0;
        }
        Ok(())
    }

    async fn bootlate(&mut self, domain: &mut BootDomain) -> Result<()> {
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_STORE_EVTCHN,
                domain.store_evtchn as u64,
            )
            .await?;
        domain
            .call
            .set_hvm_param(
                domain.domid,
                HVM_PARAM_CONSOLE_EVTCHN,
                domain.consoles[0].0 as u64,
            )
            .await?;
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
        let start_info_segment = self
            .start_info_segment
            .as_ref()
            .ok_or_else(|| Error::GenericError("start_info_segment missing".to_string()))?;
        ctx.cpu_d.typecode = 2;
        ctx.cpu_d.instance = 0;
        ctx.cpu_d.length = size_of::<HvmCpu>() as u32;
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
        ctx.cpu.rbx = start_info_segment.pfn << self.page_shift();
        ctx.cpu.dr6 = 0xffff0ff0;
        ctx.cpu.dr7 = 0x00000400;
        ctx.end_d.typecode = 0;
        ctx.end_d.instance = 0;
        ctx.end_d.length = 0;
        unsafe {
            let existing = X86PvhPlatform::get_save_record::<HvmMtrr>(&mut full_context, 14, 0);
            if existing.is_null() {
                return Err(Error::GenericError("mtrr record not found".to_string()));
            }
            let mut mtrr: MtrrCtx = MaybeUninit::zeroed().assume_init();
            mtrr.header_d = ctx.header_d;
            mtrr.header = ctx.header;
            mtrr.mtrr_d.typecode = 14;
            mtrr.mtrr_d.instance = 0;
            mtrr.mtrr_d.length = size_of::<HvmMtrr>() as u32;
            mtrr.mtrr = *existing;
            mtrr.mtrr.msr_mtrr_def_type = 6u64 | (1u64 << 11);
            mtrr.end_d.typecode = 0;
            mtrr.end_d.instance = 0;
            mtrr.end_d.length = 0;
            for i in 0..domain.max_vcpus {
                mtrr.mtrr_d.instance = i as u16;
                domain
                    .call
                    .set_hvm_context(
                        domain.domid,
                        std::slice::from_raw_parts_mut(
                            addr_of_mut!(mtrr) as *mut u8,
                            size_of::<MtrrCtx>(),
                        ),
                    )
                    .await?;
            }
        };

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
const SPECIALPAGE_IDENT_PT: u32 = 6;
const SPECIALPAGE_CONSOLE: u32 = 7;
const LAPIC_BASE_ADDRESS: u64 = 0xfee00000;
const ACPI_INFO_PHYSICAL_ADDRESS: u64 = 0xFC000000;

unsafe extern "C" fn acpi_lapic_id(cpu: libc::c_uint) -> u32 {
    cpu * 2
}

fn align_it(p: u64, a: u64) -> u64 {
    ((p) + ((a) - 1)) & !((a) - 1)
}

unsafe extern "C" fn acpi_mem_alloc(
    ctxt: *mut acpi_ctxt,
    size: u32,
    mut align: u32,
) -> *mut c_void {
    let ctx = (*ctxt).ptr as *mut AcpiBuildContext;
    if align < 16 {
        align = 16;
    }

    let s = align_it((*ctx).guest_curr, align as u64);
    let e = s + size as u64 - 1;
    if (e < s) || (e >= (*ctx).guest_end) {
        return std::ptr::null_mut();
    }
    (*ctx).guest_curr = e;
    (*ctx).buf.add((s - (*ctx).guest_start) as usize) as *mut c_void
}

unsafe extern "C" fn acpi_mem_free(_: *mut acpi_ctxt, _: *mut libc::c_void, _: u32) {}

unsafe extern "C" fn acpi_v2p(ctxt: *mut acpi_ctxt, v: *mut c_void) -> libc::c_ulong {
    let ctx = (*ctxt).ptr as *mut AcpiBuildContext;
    (*ctx).guest_start + (v as u64 - ((*ctx).buf as u64))
}

#[repr(C)]
struct AcpiBuildContext {
    page_size: u64,
    page_shift: u64,
    buf: *mut u8,
    guest_curr: u64,
    guest_start: u64,
    guest_end: u64,
}
