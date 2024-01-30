use backhand::BackhandError;
use cli_tables::TableError;
use oci_spec::OciSpecError;
use std::error::Error;
use std::ffi::NulError;
use std::fmt::{Display, Formatter};
use std::num::ParseIntError;
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

#[macro_export]
macro_rules! hypha_err {
    ($($arg:tt)*) => {{
        use $crate::error::HyphaError;
        let text = std::fmt::format(format_args!($($arg)*));
        Err(HyphaError::new(text.as_str()))
    }}
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

impl From<serde_json::Error> for HyphaError {
    fn from(value: serde_json::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<ureq::Error> for HyphaError {
    fn from(value: ureq::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<ParseIntError> for HyphaError {
    fn from(value: ParseIntError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<OciSpecError> for HyphaError {
    fn from(value: OciSpecError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<url::ParseError> for HyphaError {
    fn from(value: url::ParseError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<std::fmt::Error> for HyphaError {
    fn from(value: std::fmt::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<uuid::Error> for HyphaError {
    fn from(value: uuid::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<xenstore::error::Error> for HyphaError {
    fn from(value: xenstore::error::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<TableError> for HyphaError {
    fn from(value: TableError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<NulError> for HyphaError {
    fn from(value: NulError) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}

impl From<nix::Error> for HyphaError {
    fn from(value: nix::Error) -> Self {
        HyphaError::new(value.to_string().as_str())
    }
}
