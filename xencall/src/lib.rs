pub mod domctl;
pub mod sys;

use crate::sys::Hypercall;
use nix::errno::Errno;
use std::error::Error;
use std::ffi::{c_long, c_ulong};
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;

pub struct XenCall {
    pub handle: File,
}

#[derive(Debug)]
pub struct XenCallError {
    message: String,
}

impl XenCallError {
    pub fn new(msg: &str) -> XenCallError {
        XenCallError {
            message: msg.to_string(),
        }
    }
}

impl Display for XenCallError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for XenCallError {
    fn description(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for XenCallError {
    fn from(value: std::io::Error) -> Self {
        XenCallError::new(value.to_string().as_str())
    }
}

impl From<Errno> for XenCallError {
    fn from(value: Errno) -> Self {
        XenCallError::new(value.to_string().as_str())
    }
}

impl XenCall {
    pub fn open() -> Result<XenCall, XenCallError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/privcmd")?;
        Ok(XenCall { handle: file })
    }

    pub fn hypercall(&mut self, op: c_ulong, arg: [c_ulong; 5]) -> Result<c_long, XenCallError> {
        unsafe {
            let mut call = Hypercall { op, arg };
            let result = sys::hypercall(self.handle.as_raw_fd(), &mut call)?;
            Ok(result as c_long)
        }
    }

    pub fn hypercall0(&mut self, op: c_ulong) -> Result<c_long, XenCallError> {
        self.hypercall(op, [0, 0, 0, 0, 0])
    }

    pub fn hypercall1(&mut self, op: c_ulong, arg1: c_ulong) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, 0, 0, 0, 0])
    }

    pub fn hypercall2(
        &mut self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, 0, 0, 0])
    }

    pub fn hypercall3(
        &mut self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, 0, 0])
    }

    pub fn hypercall4(
        &mut self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, 0])
    }

    pub fn hypercall5(
        &mut self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
        arg5: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, arg5])
    }
}
