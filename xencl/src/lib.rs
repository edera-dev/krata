use std::collections::HashMap;
use std::error::Error;
use std::fmt::{Display, Formatter};
use xenstore::bus::XsdBusError;
use xenstore::client::{XsdClient, XsdInterface};

pub struct XenClient {
    store: XsdClient,
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

impl XenClient {
    pub fn open() -> Result<XenClient, XenClientError> {
        let store = XsdClient::open()?;
        Ok(XenClient { store })
    }

    pub fn create(
        &mut self,
        domid: u32,
        entries: HashMap<String, String>,
    ) -> Result<(), XenClientError> {
        let domain = self.store.get_domain_path(domid)?;
        let mut tx = self.store.transaction()?;
        for (key, value) in entries {
            let path = format!("{}/{}", domain, key);
            tx.write(path.as_str(), value.into_bytes())?;
        }
        tx.commit()?;
        Ok(())
    }
}
