pub mod error;
pub mod sys;

use error::{Error, Result};
use nix::errno::Errno;
use std::{
    fs::{File, OpenOptions},
    marker::PhantomData,
    os::{fd::AsRawFd, raw::c_void},
    sync::Arc,
    thread::sleep,
    time::Duration,
};
use sys::{
    AllocGref, DeallocGref, GetOffsetForVaddr, GrantRef, MapGrantRef, SetMaxGrants, UnmapGrantRef,
    UnmapNotify, UNMAP_NOTIFY_CLEAR_BYTE, UNMAP_NOTIFY_SEND_EVENT,
};

use libc::{mmap, munmap, MAP_FAILED, MAP_SHARED, PROT_READ, PROT_WRITE};

#[derive(Clone)]
pub struct GrantDevice {
    handle: Arc<File>,
}

impl GrantDevice {
    pub fn open() -> Result<GrantDevice> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/gntdev")?;
        Ok(GrantDevice {
            handle: Arc::new(handle),
        })
    }

    pub fn map_grant_ref(&self, refs: Vec<GrantRef>) -> Result<(u64, Vec<GrantRef>)> {
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

#[derive(Clone)]
pub struct GrantAlloc {
    handle: Arc<File>,
}

impl GrantAlloc {
    pub fn open() -> Result<GrantAlloc> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/gntalloc")?;
        Ok(GrantAlloc {
            handle: Arc::new(handle),
        })
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

#[derive(Clone)]
pub struct GrantTab {
    device: GrantDevice,
}

const PAGE_SIZE: usize = 4096;

#[allow(clippy::len_without_is_empty)]
pub struct MappedMemory<'a> {
    gnttab: GrantTab,
    length: usize,
    addr: u64,
    _ptr: PhantomData<&'a c_void>,
}

unsafe impl Send for MappedMemory<'_> {}

impl MappedMemory<'_> {
    pub fn len(&self) -> usize {
        self.length
    }

    pub fn ptr(&self) -> *mut c_void {
        self.addr as *mut c_void
    }
}

impl Drop for MappedMemory<'_> {
    fn drop(&mut self) {
        let _ = self.gnttab.unmap(self);
    }
}

impl GrantTab {
    pub fn open() -> Result<GrantTab> {
        Ok(GrantTab {
            device: GrantDevice::open()?,
        })
    }

    pub fn map_grant_refs<'a>(
        &self,
        refs: Vec<GrantRef>,
        read: bool,
        write: bool,
    ) -> Result<MappedMemory<'a>> {
        let (index, refs) = self.device.map_grant_ref(refs)?;
        unsafe {
            let mut flags: i32 = 0;
            if read {
                flags |= PROT_READ;
            }

            if write {
                flags |= PROT_WRITE;
            }

            let addr = loop {
                let addr = mmap(
                    std::ptr::null_mut(),
                    PAGE_SIZE * refs.len(),
                    flags,
                    MAP_SHARED,
                    self.device.handle.as_raw_fd(),
                    index as i64,
                );
                let errno = Errno::last();
                if addr == MAP_FAILED {
                    if errno == Errno::EAGAIN {
                        sleep(Duration::from_micros(1000));
                        continue;
                    }
                    return Err(Error::MmapFailed(errno));
                }
                break addr;
            };

            Ok(MappedMemory {
                gnttab: self.clone(),
                addr: addr as u64,
                length: PAGE_SIZE * refs.len(),
                _ptr: PhantomData,
            })
        }
    }

    fn unmap(&self, memory: &MappedMemory<'_>) -> Result<()> {
        let (offset, count) = self.device.get_offset_for_vaddr(memory.addr)?;
        let _ = unsafe { munmap(memory.addr as *mut c_void, memory.length) };
        self.device.unmap_grant_ref(offset, count)?;
        Ok(())
    }
}
