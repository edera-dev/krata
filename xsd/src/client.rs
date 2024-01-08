use crate::bus::{XsdBusError, XsdSocket};
use crate::sys::{XSD_DIRECTORY, XSD_READ};

pub struct XsdClient {
    socket: XsdSocket,
}

impl XsdClient {
    pub fn new() -> Result<XsdClient, XsdBusError> {
        let socket = XsdSocket::dial()?;
        Ok(XsdClient { socket })
    }

    pub fn list(&mut self, path: &str) -> Result<Vec<String>, XsdBusError> {
        let response = self.socket.send_single(0, XSD_DIRECTORY, path)?;
        Ok(response.parse_string_vec()?)
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, XsdBusError> {
        let response = self.socket.send_single(0, XSD_READ, path)?;
        Ok(response.payload)
    }
}
