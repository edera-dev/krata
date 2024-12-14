use nix::{ioctl_none, ioctl_readwrite_bad};
use std::ffi::c_uint;

#[repr(C)]
pub struct BindVirqRequest {
    pub virq: c_uint,
}

#[repr(C)]
pub struct BindInterdomainRequest {
    pub remote_domain: c_uint,
    pub remote_port: c_uint,
}

#[repr(C)]
pub struct BindUnboundPortRequest {
    pub remote_domain: c_uint,
}

#[repr(C)]
pub struct UnbindPortRequest {
    pub port: c_uint,
}

#[repr(C)]
pub struct NotifyRequest {
    pub port: c_uint,
}

ioctl_readwrite_bad!(bind_virq, 0x44500, BindVirqRequest);
ioctl_readwrite_bad!(bind_interdomain, 0x84501, BindInterdomainRequest);
ioctl_readwrite_bad!(bind_unbound_port, 0x44503, BindUnboundPortRequest);
ioctl_readwrite_bad!(unbind, 0x44502, UnbindPortRequest);
ioctl_readwrite_bad!(notify, 0x44504, NotifyRequest);
ioctl_none!(reset, 0x4505, 5);
