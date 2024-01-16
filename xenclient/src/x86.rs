use libc::c_char;

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
