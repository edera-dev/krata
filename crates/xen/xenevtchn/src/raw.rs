use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use byteorder::{LittleEndian, ReadBytesExt};

use crate::error::{Error, Result};
use crate::sys;

pub const EVENT_CHANNEL_DEVICE: &str = "/dev/xen/evtchn";

#[derive(Clone)]
pub struct RawEventChannelService {
    handle: Arc<Mutex<File>>,
}

impl RawEventChannelService {
    pub fn open() -> Result<RawEventChannelService> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open(EVENT_CHANNEL_DEVICE)?;
        let handle = Arc::new(Mutex::new(handle));
        Ok(RawEventChannelService { handle })
    }

    pub fn from_handle(handle: File) -> Result<RawEventChannelService> {
        Ok(RawEventChannelService {
            handle: Arc::new(Mutex::new(handle)),
        })
    }

    pub fn bind_virq(&self, virq: u32) -> Result<u32> {
        let handle = self.handle.lock().map_err(|_| Error::LockAcquireFailed)?;
        let mut request = sys::BindVirqRequest { virq };
        Ok(unsafe { sys::bind_virq(handle.as_raw_fd(), &mut request)? as u32 })
    }

    pub fn bind_interdomain(&self, domid: u32, port: u32) -> Result<u32> {
        let handle = self.handle.lock().map_err(|_| Error::LockAcquireFailed)?;
        let mut request = sys::BindInterdomainRequest {
            remote_domain: domid,
            remote_port: port,
        };
        Ok(unsafe { sys::bind_interdomain(handle.as_raw_fd(), &mut request)? as u32 })
    }

    pub fn bind_unbound_port(&self, domid: u32) -> Result<u32> {
        let handle = self.handle.lock().map_err(|_| Error::LockAcquireFailed)?;
        let mut request = sys::BindUnboundPortRequest {
            remote_domain: domid,
        };
        Ok(unsafe { sys::bind_unbound_port(handle.as_raw_fd(), &mut request)? as u32 })
    }

    pub fn unbind(&self, port: u32) -> Result<u32> {
        let handle = self.handle.lock().map_err(|_| Error::LockAcquireFailed)?;
        let mut request = sys::UnbindPortRequest { port };
        Ok(unsafe { sys::unbind(handle.as_raw_fd(), &mut request)? as u32 })
    }

    pub fn notify(&self, port: u32) -> Result<u32> {
        let handle = self.handle.lock().map_err(|_| Error::LockAcquireFailed)?;
        let mut request = sys::NotifyRequest { port };
        Ok(unsafe { sys::notify(handle.as_raw_fd(), &mut request)? as u32 })
    }

    pub fn reset(&self) -> Result<u32> {
        let handle = self.handle.lock().map_err(|_| Error::LockAcquireFailed)?;
        Ok(unsafe { sys::reset(handle.as_raw_fd())? as u32 })
    }

    pub fn pending(&self) -> Result<u32> {
        let mut handle = self.handle.lock().map_err(|_| Error::LockAcquireFailed)?;
        Ok(handle.read_u32::<LittleEndian>()?)
    }

    pub fn into_handle(self) -> Result<File> {
        Arc::into_inner(self.handle)
            .ok_or(Error::LockAcquireFailed)?
            .into_inner()
            .map_err(|_| Error::LockAcquireFailed)
    }
}
