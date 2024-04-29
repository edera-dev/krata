/// Handwritten hypercall bindings.
use nix::ioctl_readwrite_bad;
use std::ffi::{c_char, c_int, c_uint, c_ulong};
use uuid::Uuid;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct Hypercall {
    pub op: c_ulong,
    pub arg: [c_ulong; 5],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct MmapEntry {
    pub va: u64,
    pub mfn: u64,
    pub npages: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct MmapResource {
    pub dom: u16,
    pub typ: u32,
    pub id: u32,
    pub idx: u32,
    pub num: u64,
    pub addr: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MmapBatch {
    pub num: u32,
    pub domid: u16,
    pub addr: u64,
    pub mfns: *mut u64,
    pub errors: *mut c_int,
}

#[repr(C)]
#[derive(Clone, Debug)]
pub struct Mmap {
    pub num: c_int,
    pub dom: u16,
    pub entry: *mut MmapEntry,
}

const IOCTL_PRIVCMD_HYPERCALL: u64 = 0x305000;
const IOCTL_PRIVCMD_MMAP: u64 = 0x105002;
const IOCTL_PRIVCMD_MMAPBATCH_V2: u64 = 0x205004;
const IOCTL_PRIVCMD_MMAP_RESOURCE: u64 = 0x205007;

ioctl_readwrite_bad!(hypercall, IOCTL_PRIVCMD_HYPERCALL, Hypercall);
ioctl_readwrite_bad!(mmap, IOCTL_PRIVCMD_MMAP, Mmap);
ioctl_readwrite_bad!(mmapbatch, IOCTL_PRIVCMD_MMAPBATCH_V2, MmapBatch);
ioctl_readwrite_bad!(mmap_resource, IOCTL_PRIVCMD_MMAP_RESOURCE, MmapResource);

pub const HYPERVISOR_SET_TRAP_TABLE: c_ulong = 0;
pub const HYPERVISOR_MMU_UPDATE: c_ulong = 1;
pub const HYPERVISOR_SET_GDT: c_ulong = 2;
pub const HYPERVISOR_STACK_SWITCH: c_ulong = 3;
pub const HYPERVISOR_SET_CALLBACKS: c_ulong = 4;
pub const HYPERVISOR_FPU_TASKSWITCH: c_ulong = 5;
pub const HYPERVISOR_SCHED_OP_COMPAT: c_ulong = 6;
pub const HYPERVISOR_PLATFORM_OP: c_ulong = 7;
pub const HYPERVISOR_SET_DEBUGREG: c_ulong = 8;
pub const HYPERVISOR_GET_DEBUGREG: c_ulong = 9;
pub const HYPERVISOR_UPDATE_DESCRIPTOR: c_ulong = 10;
pub const HYPERVISOR_MEMORY_OP: c_ulong = 12;
pub const HYPERVISOR_MULTICALL: c_ulong = 13;
pub const HYPERVISOR_UPDATE_VA_MAPPING: c_ulong = 14;
pub const HYPERVISOR_SET_TIMER_OP: c_ulong = 15;
pub const HYPERVISOR_EVENT_CHANNEL_OP_COMPAT: c_ulong = 16;
pub const HYPERVISOR_XEN_VERSION: c_ulong = 17;
pub const HYPERVISOR_CONSOLE_IO: c_ulong = 18;
pub const HYPERVISOR_PHYSDEV_OP_COMPAT: c_ulong = 19;
pub const HYPERVISOR_GRANT_TABLE_OP: c_ulong = 20;
pub const HYPERVISOR_VM_ASSIST: c_ulong = 21;
pub const HYPERVISOR_UPDATE_VA_MAPPING_OTHERDOMAIN: c_ulong = 22;
pub const HYPERVISOR_IRET: c_ulong = 23;
pub const HYPERVISOR_VCPU_OP: c_ulong = 24;
pub const HYPERVISOR_SET_SEGMENT_BASE: c_ulong = 25;
pub const HYPERVISOR_MMUEXT_OP: c_ulong = 26;
pub const HYPERVISOR_XSM_OP: c_ulong = 27;
pub const HYPERVISOR_NMI_OP: c_ulong = 28;
pub const HYPERVISOR_SCHED_OP: c_ulong = 29;
pub const HYPERVISOR_CALLBACK_OP: c_ulong = 30;
pub const HYPERVISOR_XENOPROF_OP: c_ulong = 31;
pub const HYPERVISOR_EVENT_CHANNEL_OP: c_ulong = 32;
pub const HYPERVISOR_PHYSDEV_OP: c_ulong = 33;
pub const HYPERVISOR_HVM_OP: c_ulong = 34;
pub const HYPERVISOR_SYSCTL: c_ulong = 35;
pub const HYPERVISOR_DOMCTL: c_ulong = 36;
pub const HYPERVISOR_KEXEC_OP: c_ulong = 37;
pub const HYPERVISOR_TMEM_OP: c_ulong = 38;
pub const HYPERVISOR_XC_RESERVED_OP: c_ulong = 39;
pub const HYPERVISOR_XENPMU_OP: c_ulong = 40;
pub const HYPERVISOR_DM_OP: c_ulong = 41;

pub const XEN_DOMCTL_CDF_HVM_GUEST: u32 = 1u32 << 0;
pub const XEN_DOMCTL_CDF_HAP: u32 = 1u32 << 1;
pub const XEN_DOMCTL_CDF_S3_INTEGRITY: u32 = 1u32 << 2;
pub const XEN_DOMCTL_CDF_OOS_OFF: u32 = 1u32 << 3;
pub const XEN_DOMCTL_CDF_XS_DOMAIN: u32 = 1u32 << 4;
pub const XEN_DOMCTL_CDF_IOMMU: u32 = 1u32 << 5;

pub const XEN_X86_EMU_LAPIC: u32 = 1 << 0;
pub const XEN_X86_EMU_HPET: u32 = 1 << 1;
pub const XEN_X86_EMU_PM: u32 = 1 << 2;
pub const XEN_X86_EMU_RTC: u32 = 1 << 3;
pub const XEN_X86_EMU_IOAPIC: u32 = 1 << 4;
pub const XEN_X86_EMU_PIC: u32 = 1 << 5;
pub const XEN_X86_EMU_VGA: u32 = 1 << 6;
pub const XEN_X86_EMU_IOMMU: u32 = 1 << 7;
pub const XEN_X86_EMU_PIT: u32 = 1 << 8;
pub const XEN_X86_EMU_USE_PIRQ: u32 = 1 << 9;

pub const XEN_X86_EMU_ALL: u32 = XEN_X86_EMU_LAPIC
    | XEN_X86_EMU_HPET
    | XEN_X86_EMU_PM
    | XEN_X86_EMU_RTC
    | XEN_X86_EMU_IOAPIC
    | XEN_X86_EMU_PIC
    | XEN_X86_EMU_VGA
    | XEN_X86_EMU_IOMMU
    | XEN_X86_EMU_PIT
    | XEN_X86_EMU_USE_PIRQ;

pub const XEN_DOMCTL_CREATEDOMAIN: u32 = 1;
pub const XEN_DOMCTL_DESTROYDOMAIN: u32 = 2;
pub const XEN_DOMCTL_PAUSEDOMAIN: u32 = 3;
pub const XEN_DOMCTL_UNPAUSEDOMAIN: u32 = 4;
pub const XEN_DOMCTL_GETDOMAININFO: u32 = 5;
pub const XEN_DOMCTL_GETMEMLIST: u32 = 6;
pub const XEN_DOMCTL_SETVCPUAFFINITY: u32 = 9;
pub const XEN_DOMCTL_SHADOW_OP: u32 = 10;
pub const XEN_DOMCTL_MAX_MEM: u32 = 11;
pub const XEN_DOMCTL_SETVCPUCONTEXT: u32 = 12;
pub const XEN_DOMCTL_GETVCPUCONTEXT: u32 = 13;
pub const XEN_DOMCTL_GETVCPUINFO: u32 = 14;
pub const XEN_DOMCTL_MAX_VCPUS: u32 = 15;
pub const XEN_DOMCTL_SCHEDULER_OP: u32 = 16;
pub const XEN_DOMCTL_SETDOMAINHANDLE: u32 = 17;
pub const XEN_DOMCTL_SETDEBUGGING: u32 = 18;
pub const XEN_DOMCTL_IRQ_PERMISSION: u32 = 19;
pub const XEN_DOMCTL_IOMEM_PERMISSION: u32 = 20;
pub const XEN_DOMCTL_IOPORT_PERMISSION: u32 = 21;
pub const XEN_DOMCTL_HYPERCALL_INIT: u32 = 22;
pub const XEN_DOMCTL_SETTIMEOFFSET: u32 = 24;
pub const XEN_DOMCTL_GETVCPUAFFINITY: u32 = 25;
pub const XEN_DOMCTL_RESUMEDOMAIN: u32 = 27;
pub const XEN_DOMCTL_SENDTRIGGER: u32 = 28;
pub const XEN_DOMCTL_SUBSCRIBE: u32 = 29;
pub const XEN_DOMCTL_GETHVMCONTEXT: u32 = 33;
pub const XEN_DOMCTL_SETHVMCONTEXT: u32 = 34;
pub const XEN_DOMCTL_SET_ADDRESS_SIZE: u32 = 35;
pub const XEN_DOMCTL_GET_ADDRESS_SIZE: u32 = 36;
pub const XEN_DOMCTL_ASSIGN_DEVICE: u32 = 37;
pub const XEN_DOMCTL_BIND_PT_IRQ: u32 = 38;
pub const XEN_DOMCTL_MEMORY_MAPPING: u32 = 39;
pub const XEN_DOMCTL_IOPORT_MAPPING: u32 = 40;
pub const XEN_DOMCTL_PIN_MEM_CACHEATTR: u32 = 41;
pub const XEN_DOMCTL_SET_EXT_VCPUCONTEXT: u32 = 42;
pub const XEN_DOMCTL_GET_EXT_VCPUCONTEXT: u32 = 43;
pub const XEN_DOMCTL_TEST_ASSIGN_DEVICE: u32 = 45;
pub const XEN_DOMCTL_SET_TARGET: u32 = 46;
pub const XEN_DOMCTL_DEASSIGN_DEVICE: u32 = 47;
pub const XEN_DOMCTL_UNBIND_PT_IRQ: u32 = 48;
pub const XEN_DOMCTL_SET_CPUID: u32 = 49;
pub const XEN_DOMCTL_GET_DEVICE_GROUP: u32 = 50;
pub const XEN_DOMCTL_SET_MACHINE_ADDRESS_SIZE: u32 = 51;
pub const XEN_DOMCTL_GET_MACHINE_ADDRESS_SIZE: u32 = 52;
pub const XEN_DOMCTL_SUPPRESS_SPURIOUS_PAGE_FAULTS: u32 = 53;
pub const XEN_DOMCTL_DEBUG_OP: u32 = 54;
pub const XEN_DOMCTL_GETHVMCONTEXT_PARTIAL: u32 = 55;
pub const XEN_DOMCTL_VM_EVENT_OP: u32 = 56;
pub const XEN_DOMCTL_MEM_SHARING_OP: u32 = 57;
pub const XEN_DOMCTL_DISABLE_MIGRATE: u32 = 58;
pub const XEN_DOMCTL_GETTSCINFO: u32 = 59;
pub const XEN_DOMCTL_SETTSCINFO: u32 = 60;
pub const XEN_DOMCTL_GETPAGEFRAMEINFO3: u32 = 61;
pub const XEN_DOMCTL_SETVCPUEXTSTATE: u32 = 62;
pub const XEN_DOMCTL_GETVCPUEXTSTATE: u32 = 63;
pub const XEN_DOMCTL_SET_ACCESS_REQUIRED: u32 = 64;
pub const XEN_DOMCTL_AUDIT_P2M: u32 = 65;
pub const XEN_DOMCTL_SET_VIRQ_HANDLER: u32 = 66;
pub const XEN_DOMCTL_SET_BROKEN_PAGE_P2M: u32 = 67;
pub const XEN_DOMCTL_SETNODEAFFINITY: u32 = 68;
pub const XEN_DOMCTL_GETNODEAFFINITY: u32 = 69;
pub const XEN_DOMCTL_SET_MAX_EVTCHN: u32 = 70;
pub const XEN_DOMCTL_CACHEFLUSH: u32 = 71;
pub const XEN_DOMCTL_GET_VCPU_MSRS: u32 = 72;
pub const XEN_DOMCTL_SET_VCPU_MSRS: u32 = 73;
pub const XEN_DOMCTL_SETVNUMAINFO: u32 = 74;
pub const XEN_DOMCTL_PSR_CMT_OP: u32 = 75;
pub const XEN_DOMCTL_MONITOR_OP: u32 = 77;
pub const XEN_DOMCTL_PSR_CAT_OP: u32 = 78;
pub const XEN_DOMCTL_SOFT_RESET: u32 = 79;
pub const XEN_DOMCTL_SET_GNTTAB_LIMITS: u32 = 80;
pub const XEN_DOMCTL_VUART_OP: u32 = 81;
pub const XEN_DOMCTL_GDBSX_GUESTMEMIO: u32 = 1000;
pub const XEN_DOMCTL_GDBSX_PAUSEVCPU: u32 = 1001;
pub const XEN_DOMCTL_GDBSX_UNPAUSEVCPU: u32 = 1002;
pub const XEN_DOMCTL_GDBSX_DOMSTATUS: u32 = 1003;

#[repr(C)]
#[derive(Copy, Clone)]
pub struct DomCtl {
    pub cmd: u32,
    pub interface_version: u32,
    pub domid: u32,
    pub value: DomCtlValue,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct DomCtlVcpuContext {
    pub vcpu: u32,
    pub ctx: u64,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub struct AddressSize {
    pub size: u32,
}

#[repr(C)]
#[derive(Copy, Clone)]
pub union DomCtlValue {
    pub create_domain: CreateDomain,
    pub get_domain_info: GetDomainInfo,
    pub max_mem: MaxMem,
    pub max_cpus: MaxVcpus,
    pub hypercall_init: HypercallInit,
    pub vcpu_context: DomCtlVcpuContext,
    pub address_size: AddressSize,
    pub get_page_frame_info: GetPageFrameInfo3,
    pub ioport_permission: IoPortPermission,
    pub iomem_permission: IoMemPermission,
    pub irq_permission: IrqPermission,
    pub assign_device: AssignDevice,
    pub pad: [u8; 128],
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct CreateDomain {
    pub ssidref: u32,
    pub handle: [u8; 16],
    pub flags: u32,
    pub iommu_opts: u32,
    pub max_vcpus: u32,
    pub max_evtchn_port: u32,
    pub max_grant_frames: i32,
    pub max_maptrack_frames: i32,
    pub grant_opts: u32,
    pub vmtrace_size: u32,
    pub cpupool_id: u32,
    pub arch_domain_config: ArchDomainConfig,
}

impl Default for CreateDomain {
    fn default() -> Self {
        CreateDomain {
            ssidref: SECINITSID_DOMU,
            handle: Uuid::new_v4().into_bytes(),
            #[cfg(target_arch = "x86_64")]
            flags: 0,
            #[cfg(target_arch = "aarch64")]
            flags: 1 << XEN_DOMCTL_CDF_HVM_GUEST,
            iommu_opts: 0,
            max_vcpus: 1,
            max_evtchn_port: 1023,
            max_grant_frames: -1,
            max_maptrack_frames: -1,
            grant_opts: 2,
            vmtrace_size: 0,
            cpupool_id: 0,
            arch_domain_config: ArchDomainConfig::default(),
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct GetDomainInfo {
    pub domid: u16,
    pub pad1: u16,
    pub flags: u32,
    pub total_pages: u64,
    pub max_pages: u64,
    pub outstanding_pages: u64,
    pub shr_pages: u64,
    pub paged_pages: u64,
    pub shared_info_frame: u64,
    pub cpu_time: u64,
    pub number_online_vcpus: u32,
    pub max_vcpu_id: u32,
    pub ssidref: u32,
    pub handle: [u8; 16],
    pub cpupool: u32,
    pub gpaddr_bits: u8,
    pub pad2: [u8; 7],
    pub arch: ArchDomainConfig,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct GetPageFrameInfo3 {
    pub num: u64,
    pub array: c_ulong,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct IoPortPermission {
    pub first_port: u32,
    pub nr_ports: u32,
    pub allow: u8,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct IoMemPermission {
    pub first_mfn: u64,
    pub nr_mfns: u64,
    pub allow: u8,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct IrqPermission {
    pub pirq: u32,
    pub allow: u8,
    pub pad: [u8; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
#[cfg(target_arch = "x86_64")]
pub struct ArchDomainConfig {
    pub emulation_flags: u32,
    pub misc_flags: u32,
}

pub const X86_EMU_LAPIC: u32 = 1 << 0;

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
#[cfg(target_arch = "aarch64")]
pub struct ArchDomainConfig {
    pub gic_version: u8,
    pub sve_v1: u8,
    pub tee_type: u16,
    pub nr_spis: u32,
    pub clock_frequency: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MaxMem {
    pub max_memkb: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MaxVcpus {
    pub max_vcpus: u32,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct HypercallInit {
    pub gmfn: u64,
}

pub const XEN_DOMCTL_MIN_INTERFACE_VERSION: u32 = 0x00000015;
pub const XEN_DOMCTL_MAX_INTERFACE_VERSION: u32 = 0x00000016;

pub const SECINITSID_DOMU: u32 = 12;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct XenCapabilitiesInfo {
    pub capabilities: [c_char; 1024],
}

pub const XENVER_CAPABILITIES: u64 = 3;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MemoryReservation {
    pub extent_start: c_ulong,
    pub nr_extents: c_ulong,
    pub extent_order: c_uint,
    pub mem_flags: c_uint,
    pub domid: u16,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MultiCallEntry {
    pub op: c_ulong,
    pub result: c_ulong,
    pub args: [c_ulong; 6],
}

pub const XEN_MEM_POPULATE_PHYSMAP: u32 = 6;
pub const XEN_MEM_MEMORY_MAP: u32 = 10;
pub const XEN_MEM_SET_MEMORY_MAP: u32 = 13;
pub const XEN_MEM_CLAIM_PAGES: u32 = 24;

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct MemoryMap {
    pub count: c_uint,
    pub buffer: c_ulong,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct ForeignMemoryMap {
    pub domid: u16,
    pub map: MemoryMap,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct VcpuGuestContextFpuCtx {
    pub x: [c_char; 512],
}

impl Default for VcpuGuestContextFpuCtx {
    fn default() -> Self {
        VcpuGuestContextFpuCtx { x: [0; 512] }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
#[cfg(target_arch = "x86_64")]
pub struct CpuUserRegs {
    pub r15: u64,
    pub r14: u64,
    pub r13: u64,
    pub r12: u64,
    pub rbp: u64,
    pub rbx: u64,
    pub r11: u64,
    pub r10: u64,
    pub r9: u64,
    pub r8: u64,
    pub rax: u64,
    pub rcx: u64,
    pub rdx: u64,
    pub rsi: u64,
    pub rdi: u64,
    pub error_code: u32,
    pub entry_vector: u32,
    pub rip: u64,
    pub cs: u16,
    _pad0: [u16; 1],
    pub saved_upcall_mask: u8,
    _pad1: [u8; 3],
    pub rflags: u64,
    pub rsp: u64,
    pub ss: u16,
    _pad2: [u16; 3],
    pub es: u16,
    _pad3: [u16; 3],
    pub ds: u16,
    _pad4: [u16; 3],
    pub fs: u16,
    _pad5: [u16; 3],
    pub gs: u16,
    _pad6: [u16; 3],
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
#[cfg(target_arch = "x86_64")]
pub struct TrapInfo {
    pub vector: u8,
    pub flags: u8,
    pub cs: u16,
    pub address: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug)]
pub struct x8664VcpuGuestContext {
    pub fpu_ctx: VcpuGuestContextFpuCtx,
    pub flags: u64,
    pub user_regs: CpuUserRegs,
    pub trap_ctx: [TrapInfo; 256],
    pub ldt_base: u64,
    pub ldt_ents: u64,
    pub gdt_frames: [u64; 16],
    pub gdt_ents: u64,
    pub kernel_ss: u64,
    pub kernel_sp: u64,
    pub ctrlreg: [u64; 8],
    pub debugreg: [u64; 8],
    pub event_callback_eip: u64,
    pub failsafe_callback_eip: u64,
    pub syscall_callback_eip: u64,
    pub vm_assist: u64,
    pub fs_base: u64,
    pub gs_base_kernel: u64,
    pub gs_base_user: u64,
}

impl Default for x8664VcpuGuestContext {
    fn default() -> Self {
        Self {
            fpu_ctx: Default::default(),
            flags: 0,
            user_regs: Default::default(),
            trap_ctx: [TrapInfo::default(); 256],
            ldt_base: 0,
            ldt_ents: 0,
            gdt_frames: [0; 16],
            gdt_ents: 0,
            kernel_ss: 0,
            kernel_sp: 0,
            ctrlreg: [0; 8],
            debugreg: [0; 8],
            event_callback_eip: 0,
            failsafe_callback_eip: 0,
            syscall_callback_eip: 0,
            vm_assist: 0,
            fs_base: 0,
            gs_base_kernel: 0,
            gs_base_user: 0,
        }
    }
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct Arm64CpuUserRegs {
    pub x0: u64,
    pub x1: u64,
    pub x2: u64,
    pub x3: u64,
    pub x4: u64,
    pub x5: u64,
    pub x6: u64,
    pub x7: u64,
    pub x8: u64,
    pub x9: u64,
    pub x10: u64,
    pub x11: u64,
    pub x12: u64,
    pub x13: u64,
    pub x14: u64,
    pub x15: u64,
    pub x16: u64,
    pub x17: u64,
    pub x18: u64,
    pub x19: u64,
    pub x20: u64,
    pub x21: u64,
    pub x22: u64,
    pub x23: u64,
    pub x24: u64,
    pub x25: u64,
    pub x26: u64,
    pub x27: u64,
    pub x28: u64,
    pub x29: u64,
    pub x30: u64,
    pub pc: u64,
    pub cpsr: u64,
    pub spsr_el1: u64,
    pub spsr_fiq: u32,
    pub spsr_irq: u32,
    pub spsr_und: u32,
    pub spsr_abt: u32,
    pub sp_el0: u64,
    pub sp_el1: u64,
    pub elr_el1: u64,
}

#[repr(C)]
#[derive(Copy, Clone, Debug, Default)]
pub struct Arm64VcpuGuestContext {
    pub flags: u32,
    pub user_regs: CpuUserRegs,
    pub sctlr: u64,
    pub ttbcr: u64,
    pub ttbr0: u64,
    pub ttbr1: u64,
}

pub union VcpuGuestContextAny {
    #[cfg(target_arch = "aarch64")]
    pub value: Arm64VcpuGuestContext,
    #[cfg(target_arch = "x86_64")]
    pub value: x8664VcpuGuestContext,
}

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct MmuExtOp {
    pub cmd: c_uint,
    pub arg1: c_ulong,
    pub arg2: c_ulong,
}

pub const MMUEXT_PIN_L4_TABLE: u32 = 3;

#[repr(C)]
#[derive(Debug, Copy, Clone)]
pub struct EvtChnAllocUnbound {
    pub dom: u16,
    pub remote_dom: u16,
    pub port: u32,
}

#[repr(C, packed)]
#[derive(Debug, Copy, Clone, Default)]
pub struct E820Entry {
    pub addr: u64,
    pub size: u64,
    pub typ: u32,
}

pub const E820_MAX: u32 = 1024;
pub const E820_RAM: u32 = 1;
pub const E820_RESERVED: u32 = 2;
pub const E820_ACPI: u32 = 3;
pub const E820_NVS: u32 = 4;
pub const E820_UNUSABLE: u32 = 5;

pub const PHYSDEVOP_MAP_PIRQ: u64 = 13;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct PhysdevMapPirq {
    pub domid: u16,
    pub typ: c_int,
    pub index: c_int,
    pub pirq: c_int,
    pub bus: c_int,
    pub devfn: c_int,
    pub entry_nr: u16,
    pub table_base: u64,
}

pub const DOMCTL_DEV_RDM_RELAXED: u32 = 1;
pub const DOMCTL_DEV_PCI: u32 = 0;
pub const DOMCTL_DEV_DT: u32 = 1;

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct PciAssignDevice {
    pub sbdf: u32,
    pub padding: u64,
}

#[repr(C)]
#[derive(Default, Clone, Copy, Debug)]
pub struct AssignDevice {
    pub device: u32,
    pub flags: u32,
    pub pci_assign_device: PciAssignDevice,
}

pub const DOMID_IO: u32 = 0x7FF1;
pub const MEMFLAGS_POPULATE_ON_DEMAND: u32 = 1 << 16;

pub struct PodTarget {
    pub target_pages: u64,
    pub total_pages: u64,
    pub pod_cache_pages: u64,
    pub pod_entries: u64,
    pub domid: u16,
}
