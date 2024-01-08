use crate::sys::{XsdMessageHeader, XSD_ERROR};
use std::error::Error;
use std::ffi::{CString, FromVecWithNulError, NulError};
use std::fs::metadata;
use std::io::{Read, Write};
use std::mem::size_of;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::str::Utf8Error;
use std::string::FromUtf8Error;

const XEN_BUS_PATHS: &[&str] = &["/var/run/xenstored/socket"];

fn find_bus_path() -> Option<String> {
    for path in XEN_BUS_PATHS {
        match metadata(path) {
            Ok(_) => return Some(String::from(*path)),
            Err(_) => continue,
        }
    }
    None
}

#[derive(Debug)]
pub struct XsdBusError {
    message: String,
}

impl XsdBusError {
    pub fn new(msg: &str) -> XsdBusError {
        XsdBusError {
            message: msg.to_string(),
        }
    }
}

impl std::fmt::Display for XsdBusError {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for XsdBusError {
    fn description(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for XsdBusError {
    fn from(value: std::io::Error) -> Self {
        XsdBusError::new(value.to_string().as_str())
    }
}

impl From<NulError> for XsdBusError {
    fn from(_: NulError) -> Self {
        XsdBusError::new("Unable to coerce data into a C string.")
    }
}

impl From<FromVecWithNulError> for XsdBusError {
    fn from(_: FromVecWithNulError) -> Self {
        XsdBusError::new("Unable to coerce data into a C string.")
    }
}

impl From<Utf8Error> for XsdBusError {
    fn from(_: Utf8Error) -> Self {
        XsdBusError::new("Unable to coerce data into a UTF8 string.")
    }
}

impl From<FromUtf8Error> for XsdBusError {
    fn from(_: FromUtf8Error) -> Self {
        XsdBusError::new("Unable to coerce data into a UTF8 string.")
    }
}

pub struct XsdSocket {
    handle: UnixStream,
}

#[derive(Debug)]
pub struct XsdResponse {
    pub header: XsdMessageHeader,
    pub payload: Vec<u8>,
}

impl XsdResponse {
    pub fn parse_string_vec(&self) -> Result<Vec<String>, XsdBusError> {
        let mut strings: Vec<String> = Vec::new();
        let mut buffer: Vec<u8> = Vec::new();
        for b in &self.payload {
            if *b == 0 {
                let string = String::from_utf8(buffer.clone())?;
                strings.push(string);
                buffer.clear();
                continue;
            }
            buffer.push(*b);
        }
        Ok(strings)
    }

    pub fn parse_bool(&self) -> Result<bool, XsdBusError> {
        if self.payload.len() != 1 {
            Err(XsdBusError::new("Expected payload to be a single byte."))
        } else {
            Ok(self.payload[0] == 0)
        }
    }
}

impl XsdSocket {
    pub fn dial() -> Result<XsdSocket, XsdBusError> {
        let path = match find_bus_path() {
            Some(path) => path,
            None => return Err(XsdBusError::new("Failed to find valid bus path.")),
        };
        let stream = UnixStream::connect(path)?;
        Ok(XsdSocket { handle: stream })
    }

    pub fn send(&mut self, tx: u32, typ: u32, buf: &[u8]) -> Result<XsdResponse, XsdBusError> {
        let header = XsdMessageHeader {
            typ,
            req: 0,
            tx,
            len: buf.len() as u32,
        };
        self.handle.write_all(bytemuck::bytes_of(&header))?;
        self.handle.write_all(buf)?;
        let mut result_buf = vec![0u8; size_of::<XsdMessageHeader>()];
        self.handle.read_exact(result_buf.as_mut_slice())?;
        let result_header = bytemuck::from_bytes::<XsdMessageHeader>(&result_buf);
        let mut payload = vec![0u8; result_header.len as usize];
        self.handle.read_exact(payload.as_mut_slice())?;
        if result_header.typ == XSD_ERROR {
            let error = CString::from_vec_with_nul(payload)?;
            return Err(XsdBusError::new(error.to_str()?));
        }
        let response = XsdResponse {
            header,
            payload,
        };
        Ok(response)
    }

    pub fn send_single(
        &mut self,
        tx: u32,
        typ: u32,
        string: &str,
    ) -> Result<XsdResponse, XsdBusError> {
        let path = CString::new(string)?;
        let buf = path.as_bytes_with_nul();
        self.send(tx, typ, buf)
    }
}

impl Drop for XsdSocket {
    fn drop(&mut self) {
        self.handle.shutdown(Shutdown::Both).unwrap()
    }
}
