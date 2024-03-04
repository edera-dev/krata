use std::{
    os::fd::{AsRawFd, FromRawFd, OwnedFd},
    ptr::addr_of_mut,
};

use anyhow::Result;
use libc::{ioctl, socket, AF_INET, SOCK_DGRAM};

#[repr(C)]
struct EthtoolValue {
    cmd: u32,
    data: u32,
}

const ETHTOOL_SGSO: u32 = 0x00000024;
const ETHTOOL_STSO: u32 = 0x0000001f;
const SIOCETHTOOL: libc::c_ulong = libc::SIOCETHTOOL;

#[repr(C)]
#[derive(Debug)]
struct EthtoolIfreq {
    ifr_name: [libc::c_char; libc::IF_NAMESIZE],
    ifr_data: libc::uintptr_t,
}

impl EthtoolIfreq {
    fn new(interface: &str) -> EthtoolIfreq {
        let mut ifreq = EthtoolIfreq {
            ifr_name: [0; libc::IF_NAMESIZE],
            ifr_data: 0,
        };
        for (i, byte) in interface.as_bytes().iter().enumerate() {
            ifreq.ifr_name[i] = *byte as libc::c_char
        }
        ifreq
    }

    fn set_value(&mut self, ptr: *mut libc::c_void) {
        self.ifr_data = ptr as libc::uintptr_t;
    }
}

pub struct EthtoolHandle {
    fd: OwnedFd,
}

impl EthtoolHandle {
    pub fn new() -> Result<EthtoolHandle> {
        let fd = unsafe { socket(AF_INET, SOCK_DGRAM, 0) };
        if fd == -1 {
            return Err(std::io::Error::last_os_error().into());
        }

        Ok(EthtoolHandle {
            fd: unsafe { OwnedFd::from_raw_fd(fd) },
        })
    }

    pub fn set_gso(&mut self, interface: &str, value: bool) -> Result<()> {
        self.set_value(interface, ETHTOOL_SGSO, if value { 1 } else { 0 })
    }

    pub fn set_tso(&mut self, interface: &str, value: bool) -> Result<()> {
        self.set_value(interface, ETHTOOL_STSO, if value { 1 } else { 0 })
    }

    fn set_value(&mut self, interface: &str, cmd: u32, value: u32) -> Result<()> {
        let mut ifreq = EthtoolIfreq::new(interface);
        let mut value = EthtoolValue { cmd, data: value };
        ifreq.set_value(addr_of_mut!(value) as *mut libc::c_void);
        let result = unsafe { ioctl(self.fd.as_raw_fd(), SIOCETHTOOL, addr_of_mut!(ifreq) as u64) };
        if result == -1 {
            return Err(std::io::Error::last_os_error().into());
        }
        Ok(())
    }
}
