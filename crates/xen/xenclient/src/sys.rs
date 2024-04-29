pub const XEN_ELFNOTE_INFO: u64 = 0;
pub const XEN_ELFNOTE_ENTRY: u64 = 1;
pub const XEN_ELFNOTE_HYPERCALL_PAGE: u64 = 2;
pub const XEN_ELFNOTE_VIRT_BASE: u64 = 3;
pub const XEN_ELFNOTE_PADDR_OFFSET: u64 = 4;
pub const XEN_ELFNOTE_XEN_VERSION: u64 = 5;
pub const XEN_ELFNOTE_GUEST_OS: u64 = 6;
pub const XEN_ELFNOTE_GUEST_VERSION: u64 = 7;
pub const XEN_ELFNOTE_LOADER: u64 = 8;
pub const XEN_ELFNOTE_PAE_MODE: u64 = 9;
pub const XEN_ELFNOTE_FEATURES: u64 = 10;
pub const XEN_ELFNOTE_BSD_SYMTAB: u64 = 11;
pub const XEN_ELFNOTE_HV_START_LOW: u64 = 12;
pub const XEN_ELFNOTE_L1_MFN_VALID: u64 = 13;
pub const XEN_ELFNOTE_SUSPEND_CANCEL: u64 = 14;
pub const XEN_ELFNOTE_INIT_P2M: u64 = 15;
pub const XEN_ELFNOTE_MOD_START_PFN: u64 = 16;
pub const XEN_ELFNOTE_SUPPORTED_FEATURES: u64 = 17;
pub const XEN_ELFNOTE_PHYS32_ENTRY: u64 = 18;

#[derive(Copy, Clone)]
pub struct ElfNoteXenType {
    pub id: u64,
    pub name: &'static str,
    pub is_string: bool,
}

pub const XEN_ELFNOTE_TYPES: &[ElfNoteXenType] = &[
    ElfNoteXenType {
        id: XEN_ELFNOTE_ENTRY,
        name: "ENTRY",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_HYPERCALL_PAGE,
        name: "HYPERCALL_PAGE",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_VIRT_BASE,
        name: "VIRT_BASE",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_INIT_P2M,
        name: "INIT_P2M",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_PADDR_OFFSET,
        name: "PADDR_OFFSET",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_HV_START_LOW,
        name: "HV_START_LOW",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_XEN_VERSION,
        name: "XEN_VERSION",
        is_string: true,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_GUEST_OS,
        name: "GUEST_OS",
        is_string: true,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_GUEST_VERSION,
        name: "GUEST_VERSION",
        is_string: true,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_LOADER,
        name: "LOADER",
        is_string: true,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_PAE_MODE,
        name: "PAE_MODE",
        is_string: true,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_FEATURES,
        name: "FEATURES",
        is_string: true,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_SUPPORTED_FEATURES,
        name: "SUPPORTED_FEATURES",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_BSD_SYMTAB,
        name: "BSD_SYMTAB",
        is_string: true,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_SUSPEND_CANCEL,
        name: "SUSPEND_CANCEL",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_MOD_START_PFN,
        name: "MOD_START_PFN",
        is_string: false,
    },
    ElfNoteXenType {
        id: XEN_ELFNOTE_PHYS32_ENTRY,
        name: "PHYS32_ENTRY",
        is_string: false,
    },
];

pub const XEN_PAGE_SHIFT: u64 = 12;
pub const XEN_PAGE_SIZE: u64 = 1 << XEN_PAGE_SHIFT;
pub const XEN_PAGE_MASK: u64 = !(XEN_PAGE_SIZE - 1);
pub const SUPERPAGE_BATCH_SIZE: u64 = 512;
pub const SUPERPAGE_2MB_SHIFT: u64 = 9;
pub const SUPERPAGE_2MB_NR_PFNS: u64 = 1u64 << SUPERPAGE_2MB_SHIFT;
pub const VGCF_IN_KERNEL: u64 = 1 << 2;
pub const VGCF_ONLINE: u64 = 1 << 5;

#[repr(C)]
pub struct GrantEntry {
    pub flags: u16,
    pub domid: u16,
    pub frame: u32,
}

pub const XEN_HVM_START_MAGIC_VALUE: u64 = 0x336ec578;
