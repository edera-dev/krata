#![allow(non_camel_case_types)]

use libc::{c_char, c_int, c_void, size_t};

#[repr(C)]
pub struct libxenvchan {
    _private: [u8; 0],
}

#[repr(C)]
pub struct xentoollog_logger {
    _private: [u8; 0],
}

pub const XENVCHAN_OPEN_CLOSED: c_int = 0;
pub const XENVCHAN_OPEN_CONNECTED: c_int = 1;
pub const XENVCHAN_OPEN_WAITING_FOR_CLIENT: c_int = 2;

unsafe extern "C" {
    pub fn libxenvchan_server_init(
        logger: *mut xentoollog_logger,
        domain: c_int,
        xs_path: *const c_char,
        read_min: size_t,
        write_min: size_t,
    ) -> *mut libxenvchan;

    pub fn libxenvchan_client_init(
        logger: *mut xentoollog_logger,
        domain: c_int,
        xs_path: *const c_char,
    ) -> *mut libxenvchan;

    pub fn libxenvchan_close(ctrl: *mut libxenvchan);

    pub fn libxenvchan_recv(ctrl: *mut libxenvchan, data: *mut c_void, size: size_t) -> c_int;
    pub fn libxenvchan_read(ctrl: *mut libxenvchan, data: *mut c_void, size: size_t) -> c_int;
    pub fn libxenvchan_send(ctrl: *mut libxenvchan, data: *const c_void, size: size_t) -> c_int;
    pub fn libxenvchan_write(ctrl: *mut libxenvchan, data: *const c_void, size: size_t) -> c_int;

    pub fn libxenvchan_wait(ctrl: *mut libxenvchan) -> c_int;
    pub fn libxenvchan_fd_for_select(ctrl: *mut libxenvchan) -> c_int;
    pub fn libxenvchan_is_open(ctrl: *mut libxenvchan) -> c_int;
    pub fn libxenvchan_data_ready(ctrl: *mut libxenvchan) -> c_int;
    pub fn libxenvchan_buffer_space(ctrl: *mut libxenvchan) -> c_int;
}
