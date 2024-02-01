/// Handwritten protocol definitions for XenStore.
/// Used xen/include/public/io/xs_wire.h as a reference.
use bytemuck::{Pod, Zeroable};
use libc;

#[derive(Copy, Clone, Pod, Zeroable, Debug)]
#[repr(C)]
pub struct XsdMessageHeader {
    pub typ: u32,
    pub req: u32,
    pub tx: u32,
    pub len: u32,
}

pub const XSD_CONTROL: u32 = 0;
pub const XSD_DIRECTORY: u32 = 1;
pub const XSD_READ: u32 = 2;
pub const XSD_GET_PERMS: u32 = 3;
pub const XSD_WATCH: u32 = 4;
pub const XSD_UNWATCH: u32 = 5;
pub const XSD_TRANSACTION_START: u32 = 6;
pub const XSD_TRANSACTION_END: u32 = 7;
pub const XSD_INTRODUCE: u32 = 8;
pub const XSD_RELEASE: u32 = 9;
pub const XSD_GET_DOMAIN_PATH: u32 = 10;
pub const XSD_WRITE: u32 = 11;
pub const XSD_MKDIR: u32 = 12;
pub const XSD_RM: u32 = 13;
pub const XSD_SET_PERMS: u32 = 14;
pub const XSD_WATCH_EVENT: u32 = 15;
pub const XSD_ERROR: u32 = 16;
pub const XSD_IS_DOMAIN_INTRODUCED: u32 = 17;
pub const XSD_RESUME: u32 = 18;
pub const XSD_SET_TARGET: u32 = 19;
pub const XSD_RESET_WATCHES: u32 = XSD_SET_TARGET + 2;
pub const XSD_DIRECTORY_PART: u32 = 20;
pub const XSD_TYPE_COUNT: u32 = 21;
pub const XSD_INVALID: u32 = 0xffff;

pub const XSD_WRITE_NONE: &str = "NONE";
pub const XSD_WRITE_CREATE: &str = "CREATE";
pub const XSD_WRITE_CREATE_EXCL: &str = "CREATE|EXCL";

#[repr(C)]
pub struct XsdError<'a> {
    pub num: i32,
    pub error: &'a str,
}

pub const XSD_ERROR_EINVAL: XsdError = XsdError {
    num: libc::EINVAL,
    error: "EINVAL",
};
pub const XSD_ERROR_EACCES: XsdError = XsdError {
    num: libc::EACCES,
    error: "EACCES",
};
pub const XSD_ERROR_EEXIST: XsdError = XsdError {
    num: libc::EEXIST,
    error: "EEXIST",
};
pub const XSD_ERROR_EISDIR: XsdError = XsdError {
    num: libc::EISDIR,
    error: "EISDIR",
};
pub const XSD_ERROR_ENOENT: XsdError = XsdError {
    num: libc::ENOENT,
    error: "ENOENT",
};
pub const XSD_ERROR_ENOMEM: XsdError = XsdError {
    num: libc::ENOMEM,
    error: "ENOMEM",
};
pub const XSD_ERROR_ENOSPC: XsdError = XsdError {
    num: libc::ENOSPC,
    error: "ENOSPC",
};
pub const XSD_ERROR_EIO: XsdError = XsdError {
    num: libc::EIO,
    error: "EIO",
};
pub const XSD_ERROR_ENOTEMPTY: XsdError = XsdError {
    num: libc::ENOTEMPTY,
    error: "ENOTEMPTY",
};
pub const XSD_ERROR_ENOSYS: XsdError = XsdError {
    num: libc::ENOSYS,
    error: "ENOSYS",
};
pub const XSD_ERROR_EROFS: XsdError = XsdError {
    num: libc::EROFS,
    error: "EROFS",
};
pub const XSD_ERROR_EBUSY: XsdError = XsdError {
    num: libc::EBUSY,
    error: "EBUSY",
};
pub const XSD_ERROR_EAGAIN: XsdError = XsdError {
    num: libc::EAGAIN,
    error: "EAGAIN",
};
pub const XSD_ERROR_EISCONN: XsdError = XsdError {
    num: libc::EISCONN,
    error: "EISCONN",
};
pub const XSD_ERROR_E2BIG: XsdError = XsdError {
    num: libc::E2BIG,
    error: "E2BIG",
};
pub const XSD_ERROR_EPERM: XsdError = XsdError {
    num: libc::EPERM,
    error: "EPERM",
};

pub const XSD_WATCH_PATH: u32 = 0;
pub const XSD_WATCH_TOKEN: u32 = 1;

#[repr(C)]
pub struct XenDomainInterface {
    req: [i8; 1024],
    rsp: [i8; 1024],
    req_cons: u32,
    req_prod: u32,
    rsp_cons: u32,
    rsp_prod: u32,
    server_features: u32,
    connection: u32,
    error: u32,
}

pub const XS_PAYLOAD_MAX: u32 = 4096;
pub const XS_ABS_PATH_MAX: u32 = 3072;
pub const XS_REL_PATH_MAX: u32 = 2048;
pub const XS_SERVER_FEATURE_RECONNECTION: u32 = 1;
pub const XS_SERVER_FEATURE_ERROR: u32 = 2;
pub const XS_CONNECTED: u32 = 0;
pub const XS_RECONNECT: u32 = 1;
pub const XS_ERROR_NONE: u32 = 0;
pub const XS_ERROR_COMM: u32 = 1;
pub const XS_ERROR_RINGIDX: u32 = 2;
pub const XS_ERROR_PROTO: u32 = 3;
