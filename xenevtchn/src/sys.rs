use nix::{ioctl_none, ioctl_readwrite_bad, request_code_none};
use std::ffi::c_uint;

#[repr(C)]
pub struct BindVirq {
    pub virq: c_uint,
}

#[repr(C)]
pub struct BindInterdomain {
    pub remote_domain: c_uint,
    pub remote_port: c_uint,
}

#[repr(C)]
pub struct BindUnboundPort {
    pub remote_domain: c_uint,
}

#[repr(C)]
pub struct UnbindPort {
    pub port: c_uint,
}

#[repr(C)]
pub struct Notify {
    pub port: c_uint,
}

ioctl_readwrite_bad!(bind_virq, request_code_none!(b'E', 0), BindVirq);
ioctl_readwrite_bad!(
    bind_interdomain,
    request_code_none!(b'E', 1),
    BindInterdomain
);
ioctl_readwrite_bad!(
    bind_unbound_port,
    request_code_none!(b'E', 2),
    BindUnboundPort
);
ioctl_readwrite_bad!(unbind, request_code_none!(b'E', 3), UnbindPort);
ioctl_readwrite_bad!(notify, request_code_none!(b'E', 4), Notify);
ioctl_none!(reset, b'E', 5);
