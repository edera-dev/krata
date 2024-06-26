use libc::{c_int, ioctl};
use std::{
    fs::{File, OpenOptions},
    io,
    os::fd::{AsRawFd, IntoRawFd, RawFd},
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

#[cfg(all(not(target_os = "android"), not(target_env = "musl")))]
type IoctlRequest = libc::c_ulong;
#[cfg(any(target_os = "android", target_env = "musl"))]
type IoctlRequest = libc::c_int;

const LOOP_CONTROL: &str = "/dev/loop-control";
const LOOP_PREFIX: &str = "/dev/loop";

/// Loop control interface IOCTLs.
const LOOP_CTL_GET_FREE: IoctlRequest = 0x4C82;

/// Loop device flags.
const LO_FLAGS_READ_ONLY: u32 = 1;
const LO_FLAGS_AUTOCLEAR: u32 = 4;
const LO_FLAGS_PARTSCAN: u32 = 8;
const LO_FLAGS_DIRECT_IO: u32 = 16;

/// Loop device IOCTLs.
const LOOP_SET_FD: IoctlRequest = 0x4C00;
const LOOP_CLR_FD: IoctlRequest = 0x4C01;
const LOOP_SET_STATUS64: IoctlRequest = 0x4C04;
const LOOP_SET_CAPACITY: IoctlRequest = 0x4C07;
const LOOP_SET_DIRECT_IO: IoctlRequest = 0x4C08;

/// Interface which wraps a handle to the loop control device.
#[derive(Debug)]
pub struct LoopControl {
    dev_file: File,
}

/// Translate ioctl results to errors if appropriate.
fn translate_error(ret: i32) -> io::Result<i32> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}

impl LoopControl {
    /// Open the loop control device.
    ///
    /// # Errors
    ///
    /// Any errors from physically opening the loop control device are
    /// bubbled up.
    pub fn open() -> io::Result<Self> {
        Ok(Self {
            dev_file: OpenOptions::new()
                .read(true)
                .write(true)
                .open(LOOP_CONTROL)?,
        })
    }

    /// Requests the next available loop device from the kernel and opens it.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use krataloopdev::LoopControl;
    /// let lc = LoopControl::open().unwrap();
    /// let ld = lc.next_free().unwrap();
    /// println!("{}", ld.path().unwrap().display());
    /// ```
    ///
    /// # Errors
    ///
    /// Any errors from opening the loop device are bubbled up.
    pub fn next_free(&self) -> io::Result<LoopDevice> {
        let dev_num = translate_error(unsafe {
            ioctl(
                self.dev_file.as_raw_fd() as c_int,
                LOOP_CTL_GET_FREE as IoctlRequest,
            )
        })?;
        LoopDevice::open(format!("{}{}", LOOP_PREFIX, dev_num))
    }
}

/// Interface to a loop device itself, e.g. `/dev/loop0`.
#[derive(Debug)]
pub struct LoopDevice {
    device: File,
}

impl AsRawFd for LoopDevice {
    fn as_raw_fd(&self) -> RawFd {
        self.device.as_raw_fd()
    }
}

impl IntoRawFd for LoopDevice {
    fn into_raw_fd(self) -> RawFd {
        self.device.into_raw_fd()
    }
}

impl LoopDevice {
    /// Opens a loop device.
    ///
    /// # Errors
    ///
    /// Any errors from opening the underlying physical loop device are bubbled up.
    pub fn open<P: AsRef<Path>>(dev: P) -> io::Result<Self> {
        Ok(Self {
            device: OpenOptions::new().read(true).write(true).open(dev)?,
        })
    }

    /// Attach a loop device to a file with the given options.
    pub fn with(&self) -> AttachOptions<'_> {
        AttachOptions {
            device: self,
            info: LoopInfo64::default(),
        }
    }

    /// Enables or disables Direct I/O mode.
    pub fn set_direct_io(&self, direct_io: bool) -> io::Result<()> {
        translate_error(unsafe {
            ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_DIRECT_IO as IoctlRequest,
                if direct_io { 1 } else { 0 },
            )
        })?;
        Ok(())
    }

    /// Attach the loop device to a fully-mapped file.
    pub fn attach_file<P: AsRef<Path>>(&self, backing_file: P) -> io::Result<()> {
        let info = LoopInfo64 {
            ..Default::default()
        };

        Self::attach_with_loop_info(self, backing_file, info)
    }

    /// Attach the loop device to a file with `LoopInfo64`.
    fn attach_with_loop_info(
        &self,
        backing_file: impl AsRef<Path>,
        info: LoopInfo64,
    ) -> io::Result<()> {
        let write_access = (info.lo_flags & LO_FLAGS_READ_ONLY) == 0;
        let bf = OpenOptions::new()
            .read(true)
            .write(write_access)
            .open(backing_file)?;
        self.attach_fd_with_loop_info(bf, info)
    }

    /// Attach the loop device to a file descriptor with `LoopInfo64`.
    fn attach_fd_with_loop_info(&self, bf: impl AsRawFd, info: LoopInfo64) -> io::Result<()> {
        translate_error(unsafe {
            ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_FD as IoctlRequest,
                bf.as_raw_fd() as c_int,
            )
        })?;

        let result = unsafe {
            ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_STATUS64 as IoctlRequest,
                &info,
            )
        };

        match translate_error(result) {
            Err(err) => {
                let _detach_err = self.detach();
                Err(err)
            }
            Ok(_) => Ok(()),
        }
    }

    /// Get the path for the loop device.
    pub fn path(&self) -> Option<PathBuf> {
        let mut p = PathBuf::from("/proc/self/fd");
        p.push(self.device.as_raw_fd().to_string());
        std::fs::read_link(&p).ok()
    }

    /// Detach a loop device.
    pub fn detach(&self) -> io::Result<()> {
        translate_error(unsafe {
            ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_CLR_FD as IoctlRequest,
                0,
            )
        })?;
        Ok(())
    }

    /// Update a loop device's capacity.
    pub fn set_capacity(&self) -> io::Result<()> {
        translate_error(unsafe {
            ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_CAPACITY as IoctlRequest,
                0,
            )
        })?;
        Ok(())
    }

    /// Return the major device node number.
    pub fn major(&self) -> io::Result<u32> {
        self.device
            .metadata()
            .map(|m| unsafe { libc::major(m.rdev()) })
            .map(|m| m as u32)
    }

    /// Return the minor device node number.
    pub fn minor(&self) -> io::Result<u32> {
        self.device
            .metadata()
            .map(|m| unsafe { libc::minor(m.rdev()) })
            .map(|m| m as u32)
    }
}

#[allow(dead_code)]
#[derive(Clone)]
pub struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdevice: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

impl Default for LoopInfo64 {
    fn default() -> Self {
        Self {
            lo_device: 0,
            lo_inode: 0,
            lo_rdevice: 0,
            lo_offset: 0,
            lo_sizelimit: 0,
            lo_number: 0,
            lo_encrypt_type: 0,
            lo_encrypt_key_size: 0,
            lo_flags: 0,
            lo_file_name: [0; 64],
            lo_crypt_name: [0; 64],
            lo_encrypt_key: [0; 32],
            lo_init: [0, 2],
        }
    }
}

#[must_use]
pub struct AttachOptions<'d> {
    device: &'d LoopDevice,
    info: LoopInfo64,
}

impl AttachOptions<'_> {
    pub fn offset(mut self, offset: u64) -> Self {
        self.info.lo_offset = offset;
        self
    }

    pub fn size_limit(mut self, size_limit: u64) -> Self {
        self.info.lo_sizelimit = size_limit;
        self
    }

    pub fn read_only(mut self, read_only: bool) -> Self {
        if read_only {
            self.info.lo_flags |= LO_FLAGS_READ_ONLY;
        } else {
            self.info.lo_flags &= !LO_FLAGS_READ_ONLY;
        }
        self
    }

    pub fn autoclear(mut self, autoclear: bool) -> Self {
        if autoclear {
            self.info.lo_flags |= LO_FLAGS_AUTOCLEAR;
        } else {
            self.info.lo_flags &= !LO_FLAGS_AUTOCLEAR;
        }
        self
    }

    pub fn part_scan(mut self, part_scan: bool) -> Self {
        if part_scan {
            self.info.lo_flags |= LO_FLAGS_PARTSCAN;
        } else {
            self.info.lo_flags &= !LO_FLAGS_PARTSCAN;
        }
        self
    }

    pub fn set_direct_io(mut self, direct_io: bool) -> Self {
        if direct_io {
            self.info.lo_flags |= LO_FLAGS_DIRECT_IO;
        } else {
            self.info.lo_flags &= !LO_FLAGS_DIRECT_IO;
        }
        self
    }

    pub fn direct_io(&self) -> bool {
        if (self.info.lo_flags & LO_FLAGS_DIRECT_IO) == LO_FLAGS_DIRECT_IO {
            true
        } else {
            false
        }
    }

    pub fn attach(&self, backing_file: impl AsRef<Path>) -> io::Result<()> {
        self.device
            .attach_with_loop_info(backing_file, self.info.clone())?;
        if self.direct_io() {
            self.device.set_direct_io(self.direct_io())?;
        }
        Ok(())
    }

    pub fn attach_fd(&self, backing_file_fd: impl AsRawFd) -> io::Result<()> {
        self.device
            .attach_fd_with_loop_info(backing_file_fd, self.info.clone())?;
        if self.direct_io() {
            self.device.set_direct_io(self.direct_io())?;
        }
        Ok(())
    }
}
