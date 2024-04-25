use std::io;

use crate::pci::PciBdf;

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
    #[error("munmap failed: {0}")]
    UnmapFailed(nix::errno::Errno),
    #[error("memory setup failed: {0}")]
    MemorySetupFailed(&'static str),
    #[error("populate physmap failed: wanted={0}, received={1}, input_extents={2}")]
    PopulatePhysmapFailed(usize, usize, usize),
    #[error("unknown elf compression method")]
    ElfCompressionUnknown,
    #[error("expected elf image format not found")]
    ElfInvalidImage,
    #[error("provided elf image does not contain xen support")]
    ElfXenSupportMissing,
    #[error("regex error: {0}")]
    RegexError(#[from] regex::Error),
    #[error("error: {0}")]
    GenericError(String),
    #[error("failed to parse int: {0}")]
    ParseIntError(#[from] std::num::ParseIntError),
    #[error("invalid pci bdf string")]
    InvalidPciBdfString,
    #[error("pci device {0} is not assignable")]
    PciDeviceNotAssignable(PciBdf),
}

pub type Result<T> = std::result::Result<T, Error>;
