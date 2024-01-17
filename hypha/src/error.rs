use backhand::BackhandError;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::path::StripPrefixError;
use xenclient::XenClientError;

pub type Result<T> = std::result::Result<T, HyphaError>;

#[derive(Debug)]
pub struct HyphaError {
    message: String,
}

impl HyphaError {
    pub fn new(msg: &str) -> HyphaError {
        HyphaError {
            message: msg.to_string(),
        }
    }
}

impl Display for HyphaError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for HyphaError {
    fn description(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for HyphaError {
    fn from(value: std::io::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<XenClientError> for HyphaError {
    fn from(value: XenClientError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<ocipkg::error::Error> for HyphaError {
    fn from(value: ocipkg::error::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<walkdir::Error> for HyphaError {
    fn from(value: walkdir::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<StripPrefixError> for HyphaError {
    fn from(value: StripPrefixError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<BackhandError> for HyphaError {
    fn from(value: BackhandError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}
