use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io issue encountered: {0}")]
    Io(#[from] io::Error),
    #[error("xenstore issue encountered: {0}")]
    XenStore(#[from] xenstore::error::Error),
    #[error("xencall issue encountered: {0}")]
    XenCall(#[from] xencall::error::Error),
    #[error("domain does not have a tty")]
    TtyNotFound,
    #[error("introducing the domain failed")]
    IntroduceDomainFailed,
    #[error("string conversion of a path failed")]
    PathStringConversion,
    #[error("parent of path not found")]
    PathParentNotFound,
    #[error("domain does not exist")]
    DomainNonExistent,
    #[error("elf parse failed: {0}")]
    ElfParseFailed(#[from] elf::ParseError),
    #[error("mmap failed")]
    MmapFailed,
    #[error("munmap failed")]
    UnmapFailed,
    #[error("memory setup failed")]
    MemorySetupFailed,
    #[error("populate physmap failed: wanted={0}, received={1}, input_extents={2}")]
    PopulatePhysmapFailed(usize, usize, usize),
    #[error("unknown elf compression method")]
    ElfCompressionUnknown,
    #[error("expected elf image format not found")]
    ElfInvalidImage,
    #[error("provided elf image does not contain xen support")]
    ElfXenSupportMissing,
}

pub type Result<T> = std::result::Result<T, Error>;
