use std::fmt::{Display, Formatter};
use std::string::FromUtf8Error;
use xencall::XenCallError;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug)]
pub struct Error {
    message: String,
}

impl Error {
    pub fn new(msg: &str) -> Error {
        Error {
            message: msg.to_string(),
        }
    }
}

impl Display for Error {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for Error {
    fn description(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for Error {
    fn from(value: std::io::Error) -> Self {
        Error::new(value.to_string().as_str())
    }
}

impl From<xenstore::error::Error> for Error {
    fn from(value: xenstore::error::Error) -> Self {
        Error::new(value.to_string().as_str())
    }
}

impl From<XenCallError> for Error {
    fn from(value: XenCallError) -> Self {
        Error::new(value.to_string().as_str())
    }
}

impl From<FromUtf8Error> for Error {
    fn from(value: FromUtf8Error) -> Self {
        Error::new(value.to_string().as_str())
    }
}

impl From<xenevtchn::error::Error> for Error {
    fn from(value: xenevtchn::error::Error) -> Self {
        Error::new(value.to_string().as_str())
    }
}
