use std::ffi::{FromVecWithNulError, IntoStringError, NulError};
use std::io;
use std::num::ParseIntError;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io issue encountered")]
    Io(#[from] io::Error),
    #[error("utf8 string decode failed")]
    Utf8DecodeString(#[from] FromUtf8Error),
    #[error("utf8 str decode failed")]
    Utf8DecodeStr(#[from] Utf8Error),
    #[error("unable to decode cstring as utf8")]
    Utf8DecodeCstring(#[from] IntoStringError),
    #[error("nul byte found in string")]
    NulByteFoundString(#[from] NulError),
    #[error("unable to find nul byte in vec")]
    VecNulByteNotFound(#[from] FromVecWithNulError),
    #[error("unable to parse integer")]
    ParseInt(#[from] ParseIntError),
    #[error("bus was not found on any available path")]
    BusNotFound,
    #[error("store responded with error: `{0}`")]
    ResponseError(String),
    #[error("invalid permissions provided")]
    InvalidPermissions,
}

pub type Result<T> = std::result::Result<T, Error>;
