use crate::sys::{XsdMessageHeader, XSD_ERROR};
use std::ffi::{CString, FromVecWithNulError, IntoStringError, NulError};
use std::fs::metadata;
use std::io;
use std::io::{Read, Write};
use std::mem::size_of;
use std::net::Shutdown;
use std::num::ParseIntError;
use std::os::unix::net::UnixStream;
use std::str::Utf8Error;
use std::string::FromUtf8Error;
use thiserror::Error;

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

#[derive(Error, Debug)]
pub enum XsdBusError {
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

pub struct XsdSocket {
    handle: UnixStream,
}

#[derive(Debug)]
pub struct XsdResponse {
    pub header: XsdMessageHeader,
    pub payload: Vec<u8>,
}

impl XsdResponse {
    pub fn parse_string(&self) -> Result<String, XsdBusError> {
        Ok(CString::from_vec_with_nul(self.payload.clone())?.into_string()?)
    }

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
        Ok(true)
    }
}

impl XsdSocket {
    pub fn dial() -> Result<XsdSocket, XsdBusError> {
        let path = match find_bus_path() {
            Some(path) => path,
            None => return Err(XsdBusError::BusNotFound),
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
            return Err(XsdBusError::ResponseError(error.into_string()?));
        }
        let response = XsdResponse { header, payload };
        Ok(response)
    }

    pub fn send_single(
        &mut self,
        tx: u32,
        typ: u32,
        string: &str,
    ) -> Result<XsdResponse, XsdBusError> {
        let text = CString::new(string)?;
        let buf = text.as_bytes_with_nul();
        self.send(tx, typ, buf)
    }

    pub fn send_multiple(
        &mut self,
        tx: u32,
        typ: u32,
        array: &[&str],
    ) -> Result<XsdResponse, XsdBusError> {
        let mut buf: Vec<u8> = Vec::new();
        for item in array {
            buf.extend_from_slice(item.as_bytes());
            buf.push(0);
        }
        self.send(tx, typ, buf.as_slice())
    }
}

impl Drop for XsdSocket {
    fn drop(&mut self) {
        self.handle.shutdown(Shutdown::Both).unwrap()
    }
}
