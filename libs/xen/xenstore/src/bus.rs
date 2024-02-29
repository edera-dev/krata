use crate::error::{Error, Result};
use crate::sys::{XsdMessageHeader, XSD_ERROR};
use std::ffi::CString;
use std::fs::{metadata, File};
use std::io::{Read, Write};
use std::mem::size_of;

const XEN_BUS_PATHS: &[&str] = &["/dev/xen/xenbus"];

fn find_bus_path() -> Option<String> {
    for path in XEN_BUS_PATHS {
        match metadata(path) {
            Ok(_) => return Some(String::from(*path)),
            Err(_) => continue,
        }
    }
    None
}

pub struct XsdFileTransport {
    handle: File,
}

impl XsdFileTransport {
    fn new(path: &str) -> Result<XsdFileTransport> {
        let handle = File::options().read(true).write(true).open(path)?;
        Ok(XsdFileTransport { handle })
    }

    async fn xsd_read_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        Ok(self.handle.read_exact(buf)?)
    }

    async fn xsd_write_all(&mut self, buf: &[u8]) -> Result<()> {
        self.handle.write_all(buf)?;
        self.handle.flush()?;
        Ok(())
    }
}

pub struct XsdSocket {
    handle: XsdFileTransport,
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
    pub async fn open() -> Result<XsdSocket> {
        let path = match find_bus_path() {
            Some(path) => path,
            None => return Err(Error::BusNotFound),
        };
        let transport = XsdFileTransport::new(&path)?;
        Ok(XsdSocket { handle: transport })
    }

    pub async fn send(&mut self, tx: u32, typ: u32, buf: &[u8]) -> Result<XsdResponse> {
        let header = XsdMessageHeader {
            typ,
            req: 0,
            tx,
            len: buf.len() as u32,
        };
        let header_bytes = bytemuck::bytes_of(&header);
        let mut composed: Vec<u8> = Vec::new();
        composed.extend_from_slice(header_bytes);
        composed.extend_from_slice(buf);
        self.handle.xsd_write_all(&composed).await?;
        let mut result_buf = vec![0u8; size_of::<XsdMessageHeader>()];
        match self.handle.xsd_read_exact(result_buf.as_mut_slice()).await {
            Ok(_) => {}
            Err(error) => {
                if result_buf.first().unwrap() == &0 {
                    return Err(error);
                }
            }
        }
        let result_header = bytemuck::from_bytes::<XsdMessageHeader>(&result_buf);
        let mut payload = vec![0u8; result_header.len as usize];
        self.handle.xsd_read_exact(payload.as_mut_slice()).await?;
        if result_header.typ == XSD_ERROR {
            let error = CString::from_vec_with_nul(payload)?;
            return Err(Error::ResponseError(error.into_string()?));
        }
        let response = XsdResponse { header, payload };
        Ok(response)
    }

    pub async fn send_single(&mut self, tx: u32, typ: u32, string: &str) -> Result<XsdResponse> {
        let text = CString::new(string)?;
        let buf = text.as_bytes_with_nul();
        self.send(tx, typ, buf).await
    }

    pub async fn send_multiple(
        &mut self,
        tx: u32,
        typ: u32,
        array: &[&str],
    ) -> Result<XsdResponse> {
        let mut buf: Vec<u8> = Vec::new();
        for item in array {
            buf.extend_from_slice(item.as_bytes());
            buf.push(0);
        }
        self.send(tx, typ, buf.as_slice()).await
    }
}
