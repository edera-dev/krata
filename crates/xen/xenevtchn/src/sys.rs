use nix::{ioctl_none, ioctl_readwrite_bad};
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

ioctl_readwrite_bad!(bind_virq, 0x44500, BindVirq);
ioctl_readwrite_bad!(bind_interdomain, 0x84501, BindInterdomain);
ioctl_readwrite_bad!(bind_unbound_port, 0x44503, BindUnboundPort);
ioctl_readwrite_bad!(unbind, 0x44502, UnbindPort);
ioctl_readwrite_bad!(notify, 0x44504, Notify);
ioctl_none!(reset, 0x4505, 5);
