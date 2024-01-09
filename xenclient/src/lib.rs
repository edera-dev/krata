pub mod create;

use crate::create::DomainConfig;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::string::FromUtf8Error;
use xencall::domctl::DomainControl;
use xencall::sys::CreateDomain;
use xencall::{XenCall, XenCallError};
use xenstore::bus::XsdBusError;
use xenstore::client::{XsdClient, XsdInterface};

pub struct XenClient {
    store: XsdClient,
    call: XenCall,
}

#[derive(Debug)]
pub struct XenClientError {
    message: String,
}

impl XenClientError {
    pub fn new(msg: &str) -> XenClientError {
        XenClientError {
            message: msg.to_string(),
        }
    }
}

impl Display for XenClientError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for XenClientError {
    fn description(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for XenClientError {
    fn from(value: std::io::Error) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<XsdBusError> for XenClientError {
    fn from(value: XsdBusError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<XenCallError> for XenClientError {
    fn from(value: XenCallError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<FromUtf8Error> for XenClientError {
    fn from(value: FromUtf8Error) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl XenClient {
    pub fn open() -> Result<XenClient, XenClientError> {
        let store = XsdClient::open()?;
        let call = XenCall::open()?;
        Ok(XenClient { store, call })
    }

    pub fn create(&mut self, config: DomainConfig) -> Result<(), XenClientError> {
        let mut domctl = DomainControl::new(&mut self.call);
        let created = domctl.create_domain(CreateDomain::default())?;
        let domain = self.store.get_domain_path(created.domid)?;
        let vm = self.store.read_string(format!("{}/vm", domain).as_str())?;

        let mut tx = self.store.transaction()?;

        for (key, value) in config.clone_domain_entries() {
            let path = format!("{}/{}", domain, key);
            tx.write(path.as_str(), value.into_bytes())?;
        }

        let domid_path = format!("{}/domid", domain);
        tx.write(domid_path.as_str(), created.domid.to_string().into_bytes())?;

        for (key, value) in config.clone_domain_entries() {
            let path = format!("{}/{}", vm, key);
            tx.write(path.as_str(), value.into_bytes())?;
        }

        tx.commit()?;

        Ok(())
    }
}
