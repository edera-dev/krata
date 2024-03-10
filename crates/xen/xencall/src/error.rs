use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("version of xen is not supported")]
    XenVersionUnsupported,
    #[error("kernel error: {0}")]
    Kernel(#[from] nix::errno::Errno),
    #[error("io issue encountered: {0}")]
    Io(#[from] io::Error),
    #[error("populate physmap failed")]
    PopulatePhysmapFailed,
}

pub type Result<T> = std::result::Result<T, Error>;
