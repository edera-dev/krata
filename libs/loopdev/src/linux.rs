use std::ffi::c_uint;

pub fn major(dev: u64) -> c_uint {
    let mut major = 0;
    major |= (dev & 0x00000000000fff00) >> 8;
    major |= (dev & 0xfffff00000000000) >> 32;
    major as c_uint
}

pub fn minor(dev: u64) -> c_uint {
    let mut minor = 0;
    minor |= dev & 0x00000000000000ff;
    minor |= (dev & 0x00000ffffff00000) >> 12;
    minor as c_uint
}
