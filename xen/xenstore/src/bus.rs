use crate::error::{Error, Result};
use crate::sys::{XsdMessageHeader, XSD_ERROR};
use std::ffi::CString;
use std::fs::metadata;
use std::io::{Read, Write};
use std::mem::size_of;
use std::net::Shutdown;
use std::os::unix::net::UnixStream;

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

pub struct XsdSocket {
    handle: UnixStream,
}

#[derive(Debug)]
pub struct XsdResponse {
    pub header: XsdMessageHeader,
    pub payload: Vec<u8>,
}

impl XsdResponse {
    pub fn parse_string(&self) -> Result<String> {
        Ok(CString::from_vec_with_nul(self.payload.clone())?.into_string()?)
    }

    pub fn parse_string_vec(&self) -> Result<Vec<String>> {
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

    pub fn parse_bool(&self) -> Result<bool> {
        Ok(true)
    }
}

impl XsdSocket {
    pub fn dial() -> Result<XsdSocket> {
        let path = match find_bus_path() {
            Some(path) => path,
            None => return Err(Error::BusNotFound),
        };
        let stream = UnixStream::connect(path)?;
        Ok(XsdSocket { handle: stream })
    }

    pub fn send(&mut self, tx: u32, typ: u32, buf: &[u8]) -> Result<XsdResponse> {
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
            return Err(Error::ResponseError(error.into_string()?));
        }
        let response = XsdResponse { header, payload };
        Ok(response)
    }

    pub fn send_single(&mut self, tx: u32, typ: u32, string: &str) -> Result<XsdResponse> {
        let text = CString::new(string)?;
        let buf = text.as_bytes_with_nul();
        self.send(tx, typ, buf)
    }

    pub fn send_multiple(&mut self, tx: u32, typ: u32, array: &[&str]) -> Result<XsdResponse> {
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
