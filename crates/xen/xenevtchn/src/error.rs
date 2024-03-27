use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("kernel error: {0}")]
    Kernel(#[from] nix::errno::Errno),
    #[error("io issue encountered: {0}")]
    Io(#[from] io::Error),
    #[error("failed to send event channel wake: {0}")]
    WakeSend(tokio::sync::broadcast::error::SendError<u32>),
}

pub type Result<T> = std::result::Result<T, Error>;
