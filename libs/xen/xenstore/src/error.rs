use std::ffi::{FromVecWithNulError, IntoStringError, NulError};
use std::io;
use std::num::ParseIntError;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

use tokio::sync::mpsc::error::{SendError, TrySendError};
use tokio::sync::oneshot::error::RecvError;

use crate::bus::XsdMessage;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("io issue encountered: {0}")]
    Io(#[from] io::Error),
    #[error("invalid data received on bus")]
    InvalidBusData,
    #[error("utf8 string decode failed: {0}")]
    Utf8DecodeString(#[from] FromUtf8Error),
    #[error("utf8 str decode failed: {0}")]
    Utf8DecodeStr(#[from] Utf8Error),
    #[error("unable to decode cstring as utf8: {0}")]
    Utf8DecodeCstring(#[from] IntoStringError),
    #[error("nul byte found in string: {0}")]
    NulByteFoundString(#[from] NulError),
    #[error("unable to find nul byte in vec: {0}")]
    VecNulByteNotFound(#[from] FromVecWithNulError),
    #[error("unable to parse integer: {0}")]
    ParseInt(#[from] ParseIntError),
    #[error("bus was not found on any available path")]
    BusNotFound,
    #[error("store responded with error: `{0}`")]
    ResponseError(String),
    #[error("invalid permissions provided")]
    InvalidPermissions,
    #[error("failed to receive reply: {0}")]
    ReceiverError(#[from] RecvError),
    #[error("failed to send request: {0}")]
    SendError(#[from] SendError<XsdMessage>),
    #[error("failed to send request: {0}")]
    TrySendError(#[from] TrySendError<XsdMessage>),
}

impl Error {
    pub fn is_noent_response(&self) -> bool {
        match self {
            Error::ResponseError(message) => message == "ENOENT",
            _ => false,
        }
    }
}

pub type Result<T> = std::result::Result<T, Error>;
