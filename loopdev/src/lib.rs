// Taken from https://github.com/stratis-storage/loopdev-3/blob/master/src/lib.rs
// Licensed under MIT.

//! Setup and control loop devices.
//!
//! Provides rust interface with similar functionality to the Linux utility `losetup`.
//!
//! # Examples
//!
//! Default options:
//!
//! ```no_run
//! use loopdev::LoopControl;
//! let lc = LoopControl::open().unwrap();
//! let ld = lc.next_free().unwrap();
//!
//! println!("{}", ld.path().unwrap().display());
//!
//! ld.attach_file("disk.img").unwrap();
//! // ...
//! ld.detach().unwrap();
//! ```
//!
//! Custom options:
//!
//! ```no_run
//! # use loopdev::LoopControl;
//! # let lc = LoopControl::open().unwrap();
//! # let ld = lc.next_free().unwrap();
//! #
//! ld.with()
//!     .part_scan(true)
//!     .offset(512 * 1024 * 1024) // 512 MiB
//!     .size_limit(1024 * 1024 * 1024) // 1GiB
//!     .attach("disk.img").unwrap();
//! // ...
//! ld.detach().unwrap();
//! ```
mod bindings;
mod linux;

use crate::bindings::{
    loop_info64, LOOP_CLR_FD, LOOP_CTL_ADD, LOOP_CTL_GET_FREE, LOOP_SET_CAPACITY, LOOP_SET_FD,
    LOOP_SET_STATUS64, LO_FLAGS_AUTOCLEAR, LO_FLAGS_PARTSCAN, LO_FLAGS_READ_ONLY,
};
use libc::ioctl;
use std::ffi::{c_int, c_ulong};
use std::{
    default::Default,
    fs::{File, OpenOptions},
    io,
    os::unix::prelude::*,
    path::{Path, PathBuf},
};

#[cfg(all(not(target_os = "android"), not(target_env = "musl")))]
type IoctlRequest = c_ulong;
#[cfg(any(target_os = "android", target_env = "musl"))]
type IoctlRequest = c_int;

const LOOP_CONTROL: &str = "/dev/loop-control";
#[cfg(not(target_os = "android"))]
const LOOP_PREFIX: &str = "/dev/loop";
#[cfg(target_os = "android")]
const LOOP_PREFIX: &str = "/dev/block/loop";

/// Interface to the loop control device: `/dev/loop-control`.
#[derive(Debug)]
pub struct LoopControl {
    dev_file: File,
}

impl LoopControl {
    /// Opens the loop control device.
    ///
    /// # Errors
    ///
    /// This function will return an error for various reasons when opening
    /// the loop control file `/dev/loop-control`. See
    /// [`OpenOptions::open`](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html)
    /// for further details.
    pub fn open() -> io::Result<Self> {
        Ok(Self {
            dev_file: OpenOptions::new()
                .read(true)
                .write(true)
                .open(LOOP_CONTROL)?,
        })
    }

    /// Finds and opens the next available loop device.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use loopdev::LoopControl;
    /// let lc = LoopControl::open().unwrap();
    /// let ld = lc.next_free().unwrap();
    /// println!("{}", ld.path().unwrap().display());
    /// ```
    ///
    /// # Errors
    ///
    /// This function will return an error for various reasons when opening
    /// the loop device file `/dev/loopX`. See
    /// [`OpenOptions::open`](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html)
    /// for further details.
    pub fn next_free(&self) -> io::Result<LoopDevice> {
        let dev_num = ioctl_to_error(unsafe {
            ioctl(
                self.dev_file.as_raw_fd() as c_int,
                LOOP_CTL_GET_FREE as IoctlRequest,
            )
        })?;
        LoopDevice::open(format!("{}{}", LOOP_PREFIX, dev_num))
    }

    /// Add and opens a new loop device.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use loopdev::LoopControl;
    /// let lc = LoopControl::open().unwrap();
    /// let ld = lc.add(1).unwrap();
    /// println!("{}", ld.path().unwrap().display());
    /// ```
    ///
    /// # Errors
    ///
    /// This funcitons will return an error when a loop device with the passed
    /// number exists or opening the newly created device fails.
    pub fn add(&self, n: u32) -> io::Result<LoopDevice> {
        let dev_num = ioctl_to_error(unsafe {
            ioctl(
                self.dev_file.as_raw_fd() as c_int,
                LOOP_CTL_ADD as IoctlRequest,
                n as c_int,
            )
        })?;
        LoopDevice::open(format!("{}{}", LOOP_PREFIX, dev_num))
    }
}

impl AsRawFd for LoopControl {
    fn as_raw_fd(&self) -> RawFd {
        self.dev_file.as_raw_fd()
    }
}

impl IntoRawFd for LoopControl {
    fn into_raw_fd(self) -> RawFd {
        self.dev_file.into_raw_fd()
    }
}

/// Interface to a loop device ie `/dev/loop0`.
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
    /// This function will return an error for various reasons when opening
    /// the given loop device file. See
    /// [`OpenOptions::open`](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html)
    /// for further details.
    pub fn open<P: AsRef<Path>>(dev: P) -> io::Result<Self> {
        // TODO create dev if it does not exist and begins with LOOP_PREFIX
        Ok(Self {
            device: OpenOptions::new().read(true).write(true).open(dev)?,
        })
    }

    /// Attach the loop device to a file with given options.
    ///
    /// # Examples
    ///
    /// Attach the device to a file.
    ///
    /// ```no_run
    /// use loopdev::LoopDevice;
    /// let mut ld = LoopDevice::open("/dev/loop0").unwrap();
    /// ld.with().part_scan(true).attach("disk.img").unwrap();
    /// # ld.detach().unwrap();
    /// ```
    pub fn with(&self) -> AttachOptions<'_> {
        AttachOptions {
            device: self,
            info: loop_info64::default(),
        }
    }

    /// Attach the loop device to a file that maps to the whole file.
    ///
    /// # Examples
    ///
    /// Attach the device to a file.
    ///
    /// ```no_run
    /// use loopdev::LoopDevice;
    /// let ld = LoopDevice::open("/dev/loop0").unwrap();
    /// ld.attach_file("disk.img").unwrap();
    /// # ld.detach().unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// This function will return an error for various reasons. Either when
    /// opening the backing file (see
    /// [`OpenOptions::open`](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html)
    /// for further details) or when calling the ioctl to attach the backing
    /// file to the device.
    pub fn attach_file<P: AsRef<Path>>(&self, backing_file: P) -> io::Result<()> {
        let info = loop_info64 {
            ..Default::default()
        };

        Self::attach_with_loop_info(self, backing_file, info)
    }

    /// Attach the loop device to a file with `loop_info64`.
    fn attach_with_loop_info(
        &self, // TODO should be mut? - but changing it is a breaking change
        backing_file: impl AsRef<Path>,
        info: loop_info64,
    ) -> io::Result<()> {
        let write_access = (info.lo_flags & LO_FLAGS_READ_ONLY) == 0;
        let bf = OpenOptions::new()
            .read(true)
            .write(write_access)
            .open(backing_file)?;
        self.attach_fd_with_loop_info(bf, info)
    }

    /// Attach the loop device to a fd with `loop_info`.
    fn attach_fd_with_loop_info(&self, bf: impl AsRawFd, info: loop_info64) -> io::Result<()> {
        // Attach the file
        ioctl_to_error(unsafe {
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
        match ioctl_to_error(result) {
            Err(err) => {
                // Ignore the error to preserve the original error
                let _detach_err = self.detach();
                Err(err)
            }
            Ok(_) => Ok(()),
        }
    }

    /// Get the path of the loop device.
    pub fn path(&self) -> Option<PathBuf> {
        let mut p = PathBuf::from("/proc/self/fd");
        p.push(self.device.as_raw_fd().to_string());
        std::fs::read_link(&p).ok()
    }

    /// Get the device major number
    ///
    /// # Errors
    ///
    /// This function needs to stat the backing file and can fail if there is
    /// an IO error.
    #[allow(clippy::unnecessary_cast)]
    pub fn major(&self) -> io::Result<u32> {
        self.device
            .metadata()
            .map(|m| linux::major(m.rdev()))
            .map(|m| m as u32)
    }

    /// Get the device major number
    ///
    /// # Errors
    ///
    /// This function needs to stat the backing file and can fail if there is
    /// an IO error.
    #[allow(clippy::unnecessary_cast)]
    pub fn minor(&self) -> io::Result<u32> {
        self.device
            .metadata()
            .map(|m| linux::minor(m.rdev()))
            .map(|m| m as u32)
    }

    /// Detach a loop device from its backing file.
    ///
    /// Note that the device won't fully detach until a short delay after the underling device file
    /// gets closed. This happens when `LoopDev` goes out of scope so you should ensure the `LoopDev`
    /// lives for a short a time as possible.
    ///
    /// # Examples
    ///
    /// ```no_run
    /// use loopdev::LoopDevice;
    /// let ld = LoopDevice::open("/dev/loop0").unwrap();
    /// # ld.attach_file("disk.img").unwrap();
    /// ld.detach().unwrap();
    /// ```
    ///
    /// # Errors
    ///
    /// This function will return an error for various reasons when calling the
    /// ioctl to detach the backing file from the device.
    pub fn detach(&self) -> io::Result<()> {
        ioctl_to_error(unsafe {
            ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_CLR_FD as IoctlRequest,
                0,
            )
        })?;
        Ok(())
    }

    /// Resize a live loop device. If the size of the backing file changes this can be called to
    /// inform the loop driver about the new size.
    ///
    /// # Errors
    ///
    /// This function will return an error for various reasons when calling the
    /// ioctl to set the capacity of the device.
    pub fn set_capacity(&self) -> io::Result<()> {
        ioctl_to_error(unsafe {
            ioctl(
                self.device.as_raw_fd() as c_int,
                LOOP_SET_CAPACITY as IoctlRequest,
                0,
            )
        })?;
        Ok(())
    }
}

/// Used to set options when attaching a device. Created with [`LoopDevice::with`()].
///
/// # Examples
///
/// Enable partition scanning on attach:
///
/// ```no_run
/// use loopdev::LoopDevice;
/// let mut ld = LoopDevice::open("/dev/loop0").unwrap();
/// ld.with()
///     .part_scan(true)
///     .attach("disk.img")
///     .unwrap();
/// # ld.detach().unwrap();
/// ```
///
/// A 1MiB slice of the file located at 1KiB into the file.
///
/// ```no_run
/// use loopdev::LoopDevice;
/// let mut ld = LoopDevice::open("/dev/loop0").unwrap();
/// ld.with()
///     .offset(1024*1024)
///     .size_limit(1024*1024*1024)
///     .attach("disk.img")
///     .unwrap();
/// # ld.detach().unwrap();
/// ```
#[must_use]
pub struct AttachOptions<'d> {
    device: &'d LoopDevice,
    info: loop_info64,
}

impl AttachOptions<'_> {
    /// Offset in bytes from the start of the backing file the data will start at.
    pub fn offset(mut self, offset: u64) -> Self {
        self.info.lo_offset = offset;
        self
    }

    /// Maximum size of the data in bytes.
    pub fn size_limit(mut self, size_limit: u64) -> Self {
        self.info.lo_sizelimit = size_limit;
        self
    }

    /// Set read only flag
    pub fn read_only(mut self, read_only: bool) -> Self {
        if read_only {
            self.info.lo_flags |= LO_FLAGS_READ_ONLY;
        } else {
            self.info.lo_flags &= !LO_FLAGS_READ_ONLY;
        }
        self
    }

    /// Set autoclear flag
    pub fn autoclear(mut self, autoclear: bool) -> Self {
        if autoclear {
            self.info.lo_flags |= LO_FLAGS_AUTOCLEAR;
        } else {
            self.info.lo_flags &= !LO_FLAGS_AUTOCLEAR;
        }
        self
    }

    /// Force the kernel to scan the partition table on a newly created loop device. Note that the
    /// partition table parsing depends on sector sizes. The default is sector size is 512 bytes
    pub fn part_scan(mut self, enable: bool) -> Self {
        if enable {
            self.info.lo_flags |= LO_FLAGS_PARTSCAN;
        } else {
            self.info.lo_flags &= !LO_FLAGS_PARTSCAN;
        }
        self
    }

    /// Attach the loop device to a file with the set options.
    ///
    /// # Errors
    ///
    /// This function will return an error for various reasons. Either when
    /// opening the backing file (see
    /// [`OpenOptions::open`](https://doc.rust-lang.org/std/fs/struct.OpenOptions.html)
    /// for further details) or when calling the ioctl to attach the backing
    /// file to the device.
    pub fn attach(self, backing_file: impl AsRef<Path>) -> io::Result<()> {
        self.device.attach_with_loop_info(backing_file, self.info)?;
        Ok(())
    }

    /// Attach the loop device to an fd
    ///
    /// # Errors
    ///
    /// This function will return an error for various reasons when calling the
    /// ioctl to attach the backing file to the device.
    pub fn attach_fd(self, backing_file_fd: impl AsRawFd) -> io::Result<()> {
        self.device
            .attach_fd_with_loop_info(backing_file_fd, self.info)?;
        Ok(())
    }
}

fn ioctl_to_error(ret: i32) -> io::Result<i32> {
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(ret)
    }
}
