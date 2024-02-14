use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("kernel error")]
    Kernel(#[from] nix::errno::Errno),
    #[error("io issue encountered")]
    Io(#[from] io::Error),
    #[error("failed to send event channel wake")]
    WakeSend(tokio::sync::broadcast::error::SendError<u32>),
}

pub type Result<T> = std::result::Result<T, Error>;
