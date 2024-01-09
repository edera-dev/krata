use nix::{ioctl_readwrite_bad, request_code_none};
use std::ffi::{c_long, c_ulong};

#[repr(C)]
pub struct Hypercall {
    pub op: c_ulong,
    pub arg: [c_ulong; 5],
    pub retval: c_long,
}

ioctl_readwrite_bad!(hypercall, request_code_none!(b'E', 0), Hypercall);
