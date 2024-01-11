pub mod domctl;
pub mod memory;
pub mod sys;

use crate::sys::{
    Hypercall, MmapBatch, XenCapabilitiesInfo, HYPERVISOR_XEN_VERSION, XENVER_CAPABILITIES,
};
use libc::{mmap, MAP_FAILED, MAP_SHARED, PROT_READ, PROT_WRITE};
use nix::errno::Errno;
use std::error::Error;
use std::ffi::{c_long, c_ulong, c_void};
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::ptr::addr_of_mut;

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

    pub fn mmap(&self, addr: u64, len: u64) -> Option<u64> {
        unsafe {
            let ptr = mmap(
                addr as *mut c_void,
                len as usize,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                self.handle.as_raw_fd(),
                0,
            );
            if ptr == MAP_FAILED {
                None
            } else {
                Some(ptr as u64)
            }
        }
    }

    pub fn hypercall(&self, op: c_ulong, arg: [c_ulong; 5]) -> Result<c_long, XenCallError> {
        unsafe {
            let mut call = Hypercall { op, arg };
            let result = sys::hypercall(self.handle.as_raw_fd(), &mut call)?;
            Ok(result as c_long)
        }
    }

    pub fn hypercall0(&self, op: c_ulong) -> Result<c_long, XenCallError> {
        self.hypercall(op, [0, 0, 0, 0, 0])
    }

    pub fn hypercall1(&self, op: c_ulong, arg1: c_ulong) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, 0, 0, 0, 0])
    }

    pub fn hypercall2(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, 0, 0, 0])
    }

    pub fn hypercall3(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, 0, 0])
    }

    pub fn hypercall4(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, 0])
    }

    pub fn hypercall5(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
        arg5: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, arg5])
    }

    pub fn mmap_batch(
        &self,
        domid: u32,
        num: u64,
        addr: u64,
        mfns: Vec<u64>,
    ) -> Result<c_long, XenCallError> {
        unsafe {
            let mut mfns = mfns.clone();
            let mut errors = vec![0i32; mfns.len()];
            let mut batch = MmapBatch {
                num: num as u32,
                domid: domid as u16,
                addr,
                mfns: mfns.as_mut_ptr(),
                errors: errors.as_mut_ptr(),
            };
            let result = sys::mmapbatch(self.handle.as_raw_fd(), &mut batch)?;
            Ok(result as c_long)
        }
    }

    pub fn get_version_capabilities(&self) -> Result<XenCapabilitiesInfo, XenCallError> {
        let mut info = XenCapabilitiesInfo {
            capabilities: [0; 1024],
        };
        self.hypercall2(
            HYPERVISOR_XEN_VERSION,
            XENVER_CAPABILITIES,
            addr_of_mut!(info) as c_ulong,
        )?;
        Ok(info)
    }
}
