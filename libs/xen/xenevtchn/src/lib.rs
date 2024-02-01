pub mod error;
pub mod sys;

use crate::error::Result;
use crate::sys::{BindInterdomain, BindUnboundPort, BindVirq, Notify, UnbindPort};

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

pub struct EventChannel {
    pub handle: File,
}

impl EventChannel {
    pub fn open() -> Result<EventChannel> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/evtchn")?;
        Ok(EventChannel { handle: file })
    }

    pub fn bind_virq(&mut self, virq: u32) -> Result<u32> {
        unsafe {
            let mut request = BindVirq { virq };
            Ok(sys::bind_virq(self.handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub fn bind_interdomain(&mut self, domid: u32, port: u32) -> Result<u32> {
        unsafe {
            let mut request = BindInterdomain {
                remote_domain: domid,
                remote_port: port,
            };
            Ok(sys::bind_interdomain(self.handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub fn bind_unbound_port(&mut self, domid: u32) -> Result<u32> {
        unsafe {
            let mut request = BindUnboundPort {
                remote_domain: domid,
            };
            Ok(sys::bind_unbound_port(self.handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub fn unbind(&mut self, port: u32) -> Result<u32> {
        unsafe {
            let mut request = UnbindPort { port };
            Ok(sys::unbind(self.handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub fn notify(&mut self, port: u32) -> Result<u32> {
        unsafe {
            let mut request = Notify { port };
            Ok(sys::notify(self.handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub fn reset(&mut self) -> Result<u32> {
        unsafe { Ok(sys::reset(self.handle.as_raw_fd())? as u32) }
    }
}
