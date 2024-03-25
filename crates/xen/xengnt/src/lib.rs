pub mod error;
pub mod sys;

use error::{Error, Result};
use std::{
    fs::{File, OpenOptions},
    os::fd::AsRawFd,
};
use sys::{
    AllocGref, DeallocGref, GetOffsetForVaddr, GrantRef, MapGrantRef, SetMaxGrants, UnmapGrantRef,
    UnmapNotify, UNMAP_NOTIFY_CLEAR_BYTE, UNMAP_NOTIFY_SEND_EVENT,
};

pub struct GrantDevice {
    handle: File,
}

impl GrantDevice {
    pub fn open() -> Result<GrantDevice> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/gntdev")?;
        Ok(GrantDevice { handle })
    }

    pub fn map_grant_ref(&self, count: u32) -> Result<(u64, Vec<GrantRef>)> {
        let refs: Vec<GrantRef> = vec![
            GrantRef {
                domid: 0,
                reference: 0
            };
            count as usize
        ];
        let mut request = MapGrantRef::write(refs.as_slice());
        unsafe {
            sys::map_grant_ref(self.handle.as_raw_fd(), request.as_mut_ptr())?;
        };
        let result =
            MapGrantRef::read(refs.len() as u32, request).ok_or(Error::StructureReadFailed)?;
        Ok((result.index, result.refs))
    }

    pub fn unmap_grant_ref(&self, index: u64, count: u32) -> Result<()> {
        let mut request = UnmapGrantRef {
            index,
            count,
            pad: 0,
        };
        unsafe {
            sys::unmap_grant_ref(self.handle.as_raw_fd(), &mut request)?;
        }
        Ok(())
    }

    pub fn get_offset_for_vaddr(&self, vaddr: u64) -> Result<(u64, u32)> {
        let mut request = GetOffsetForVaddr {
            vaddr,
            pad: 0,
            offset: 0,
            count: 0,
        };
        unsafe {
            sys::get_offset_for_vaddr(self.handle.as_raw_fd(), &mut request)?;
        }
        Ok((request.offset, request.count))
    }

    pub fn set_max_grants(&self, count: u32) -> Result<()> {
        let mut request = SetMaxGrants { count };
        unsafe {
            sys::set_max_grants(self.handle.as_raw_fd(), &mut request)?;
        }
        Ok(())
    }

    pub fn unmap_notify(&self, index: u64, send: bool, port: u32) -> Result<()> {
        let mut request = UnmapNotify {
            index,
            action: if send {
                UNMAP_NOTIFY_SEND_EVENT
            } else {
                UNMAP_NOTIFY_CLEAR_BYTE
            },
            port,
        };
        unsafe {
            sys::unmap_notify(self.handle.as_raw_fd(), &mut request)?;
        }
        Ok(())
    }
}

pub struct GrantAlloc {
    handle: File,
}

impl GrantAlloc {
    pub fn open() -> Result<GrantAlloc> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/gntalloc")?;
        Ok(GrantAlloc { handle })
    }

    pub fn alloc_gref(&self, domid: u16, flags: u16, count: u32) -> Result<(u64, Vec<u32>)> {
        let mut request = AllocGref::write(AllocGref {
            domid,
            flags,
            count,
        });
        unsafe {
            sys::alloc_gref(self.handle.as_raw_fd(), request.as_mut_ptr())?;
        };
        AllocGref::read(count, request).ok_or(Error::StructureReadFailed)
    }

    pub fn dealloc_gref(&self, index: u64, count: u32) -> Result<()> {
        let mut request = DeallocGref { index, count };
        unsafe {
            sys::dealloc_gref(self.handle.as_raw_fd(), &mut request)?;
        };
        Ok(())
    }

    pub fn unmap_notify(&self, index: u64, send: bool, port: u32) -> Result<()> {
        let mut request = UnmapNotify {
            index,
            action: if send {
                UNMAP_NOTIFY_SEND_EVENT
            } else {
                UNMAP_NOTIFY_CLEAR_BYTE
            },
            port,
        };
        unsafe {
            sys::unmap_notify(self.handle.as_raw_fd(), &mut request)?;
        }
        Ok(())
    }
}
