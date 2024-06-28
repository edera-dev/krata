use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("version of xen is not supported")]
    XenVersionUnsupported,
    #[error("kernel error: {0}")]
    Kernel(#[from] nix::errno::Errno),
    #[error("io issue encountered: {0}")]
    Io(#[from] io::Error),
    #[error("failed to acquire semaphore: {0}")]
    AcquireSemaphoreFailed(#[from] tokio::sync::AcquireError),
    #[error("populate physmap failed")]
    PopulatePhysmapFailed,
    #[error("mmap batch failed: {0}")]
    MmapBatchFailed(nix::errno::Errno),
    #[error("specified value is too long")]
    ValueTooLong,
}

pub type Result<T> = std::result::Result<T, Error>;
