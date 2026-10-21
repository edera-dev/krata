use std::ffi::NulError;
use std::io;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("{op} failed: {source}")]
    Api {
        op: &'static str,
        #[source]
        source: io::Error,
    },
    #[error("xenstore path contains an interior NUL byte")]
    InvalidPath(#[from] NulError),
    #[error("vchan peer is closed")]
    Closed,
}

pub type Result<T> = std::result::Result<T, Error>;
