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
