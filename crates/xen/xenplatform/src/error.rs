use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io issue encountered: {0}")]
    Io(#[from] io::Error),
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
    #[error("munmap failed: {0}")]
    UnmapFailed(nix::errno::Errno),
    #[error("memory setup failed: {0}")]
    MemorySetupFailed(&'static str),
    #[error("populate physmap failed: wanted={0}, received={1}, input_extents={2}")]
    PopulatePhysmapFailed(usize, usize, usize),
    #[error("unknown elf compression method")]
    ElfCompressionUnknown,
    #[error("elf image format invalid: {0}")]
    ElfInvalidImage(&'static str),
    #[error("elf linux image not found")]
    ElfNotLinux,
    #[error("provided elf image does not contain xen support")]
    ElfXenSupportMissing,
    #[error("provided elf image does not contain xen note {0}")]
    ElfXenNoteMissing(&'static str),
    #[error("regex error: {0}")]
    RegexError(#[from] regex::Error),
    #[error("error: {0}")]
    GenericError(String),
    #[error("failed to parse int: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("failed to join async task: {0}")]
    AsyncJoinError(#[from] tokio::task::JoinError),
}

pub type Result<T> = std::result::Result<T, Error>;
