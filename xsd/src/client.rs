use crate::bus::{XsdBusError, XsdSocket};
use crate::sys::{XSD_DIRECTORY, XSD_MKDIR, XSD_READ, XSD_RM, XSD_WRITE};
use std::ffi::CString;

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
        response.parse_string_vec()
    }

    pub fn read(&mut self, path: &str) -> Result<Vec<u8>, XsdBusError> {
        let response = self.socket.send_single(0, XSD_READ, path)?;
        Ok(response.payload)
    }

    pub fn write(&mut self, path: &str, data: Vec<u8>) -> Result<bool, XsdBusError> {
        let mut buffer = Vec::new();
        let path = CString::new(path)?;
        buffer.extend_from_slice(path.as_bytes_with_nul());
        buffer.extend_from_slice(data.as_slice());
        let response = self.socket.send(0, XSD_WRITE, buffer.as_slice())?;
        response.parse_bool()
    }

    pub fn mkdir(&mut self, path: &str) -> Result<bool, XsdBusError> {
        self.socket.send_single(0, XSD_MKDIR, path)?.parse_bool()
    }

    pub fn rm(&mut self, path: &str) -> Result<bool, XsdBusError> {
        self.socket.send_single(0, XSD_RM, path)?.parse_bool()
    }
}
