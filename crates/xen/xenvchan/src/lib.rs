pub mod error;

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;
use std::ptr::NonNull;

use error::{Error, Result};
use xenvchan_sys::{
    libxenvchan, libxenvchan_buffer_space, libxenvchan_client_init, libxenvchan_close,
    libxenvchan_data_ready, libxenvchan_fd_for_select, libxenvchan_is_open, libxenvchan_read,
    libxenvchan_recv, libxenvchan_send, libxenvchan_server_init, libxenvchan_wait,
    libxenvchan_write, XENVCHAN_OPEN_CLOSED, XENVCHAN_OPEN_CONNECTED,
    XENVCHAN_OPEN_WAITING_FOR_CLIENT,
};

#[derive(Debug, Clone)]
pub struct VchanConfig {
    pub domid: u32,
    pub xs_path: String,
    pub read_min: usize,
    pub write_min: usize,
}

impl VchanConfig {
    pub fn symmetric(domid: u32, xs_path: impl Into<String>, ring_min: usize) -> Self {
        Self {
            domid,
            xs_path: xs_path.into(),
            read_min: ring_min,
            write_min: ring_min,
        }
    }
}

struct RawVchan {
    ptr: NonNull<libxenvchan>,
}

// SAFETY: The underlying libxenvchan handle is an owned FFI resource. We only
// move it between tasks/threads and serialize all operations through &mut self
// or an outer Mutex in isol8-vchan.
unsafe impl Send for RawVchan {}

impl RawVchan {
    fn server(cfg: &VchanConfig) -> Result<Self> {
        let xs_path = CString::new(cfg.xs_path.as_str())?;
        let ptr = unsafe {
            libxenvchan_server_init(
                std::ptr::null_mut(),
                cfg.domid as i32,
                xs_path.as_ptr(),
                cfg.read_min,
                cfg.write_min,
            )
        };
        Self::from_init_ptr("libxenvchan_server_init", ptr)
    }

    fn client(cfg: &VchanConfig) -> Result<Self> {
        let xs_path = CString::new(cfg.xs_path.as_str())?;
        let ptr = unsafe {
            libxenvchan_client_init(std::ptr::null_mut(), cfg.domid as i32, xs_path.as_ptr())
        };
        Self::from_init_ptr("libxenvchan_client_init", ptr)
    }

    fn from_init_ptr(op: &'static str, ptr: *mut libxenvchan) -> Result<Self> {
        let ptr = NonNull::new(ptr).ok_or_else(|| last_error(op))?;
        Ok(Self { ptr })
    }

    fn as_ptr(&self) -> *mut libxenvchan {
        self.ptr.as_ptr()
    }

    fn wait_connected(&self) -> Result<()> {
        loop {
            match self.open_state()? {
                XENVCHAN_OPEN_CONNECTED => return Ok(()),
                XENVCHAN_OPEN_WAITING_FOR_CLIENT => self.wait()?,
                XENVCHAN_OPEN_CLOSED => return Err(Error::Closed),
                _ => self.wait()?,
            }
        }
    }

    fn wait(&self) -> Result<()> {
        let rc = unsafe { libxenvchan_wait(self.as_ptr()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_wait"));
        }
        Ok(())
    }

    fn open_state(&self) -> Result<i32> {
        let rc = unsafe { libxenvchan_is_open(self.as_ptr()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_is_open"));
        }
        Ok(rc)
    }

    fn fd_for_select(&self) -> Result<RawFd> {
        let rc = unsafe { libxenvchan_fd_for_select(self.as_ptr()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_fd_for_select"));
        }
        Ok(rc)
    }

    fn data_ready(&self) -> Result<usize> {
        let rc = unsafe { libxenvchan_data_ready(self.as_ptr()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_data_ready"));
        }
        Ok(rc as usize)
    }

    fn buffer_space(&self) -> Result<usize> {
        let rc = unsafe { libxenvchan_buffer_space(self.as_ptr()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_buffer_space"));
        }
        Ok(rc as usize)
    }

    fn read_some(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            let rc = unsafe { libxenvchan_read(self.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
            if rc > 0 {
                return Ok(rc as usize);
            }
            if rc < 0 {
                return Err(last_error("libxenvchan_read"));
            }
            if self.open_state()? == XENVCHAN_OPEN_CLOSED {
                return Ok(0);
            }
            self.wait()?;
        }
    }

    fn write_some(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        loop {
            let rc = unsafe { libxenvchan_write(self.as_ptr(), buf.as_ptr().cast(), buf.len()) };
            if rc > 0 {
                return Ok(rc as usize);
            }
            if rc < 0 {
                return Err(last_error("libxenvchan_write"));
            }
            if self.open_state()? == XENVCHAN_OPEN_CLOSED {
                return Err(Error::Closed);
            }
            self.wait()?;
        }
    }

    fn recv_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }

        loop {
            let rc = unsafe { libxenvchan_recv(self.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
            if rc == buf.len() as i32 {
                return Ok(());
            }
            if rc < 0 {
                return Err(last_error("libxenvchan_recv"));
            }
            if self.open_state()? == XENVCHAN_OPEN_CLOSED {
                return Err(Error::Closed);
            }
            self.wait()?;
        }
    }

    fn send_exact(&mut self, buf: &[u8]) -> Result<()> {
        if buf.is_empty() {
            return Ok(());
        }

        loop {
            let rc = unsafe { libxenvchan_send(self.as_ptr(), buf.as_ptr().cast(), buf.len()) };
            if rc == buf.len() as i32 {
                return Ok(());
            }
            if rc < 0 {
                return Err(last_error("libxenvchan_send"));
            }
            if self.open_state()? == XENVCHAN_OPEN_CLOSED {
                return Err(Error::Closed);
            }
            self.wait()?;
        }
    }
}

impl Drop for RawVchan {
    fn drop(&mut self) {
        unsafe { libxenvchan_close(self.as_ptr()) };
    }
}

pub struct VchanListener {
    raw: RawVchan,
}

impl VchanListener {
    pub fn bind(cfg: &VchanConfig) -> Result<Self> {
        Ok(Self {
            raw: RawVchan::server(cfg)?,
        })
    }

    pub fn accept(self) -> Result<VchanStream> {
        self.raw.wait_connected()?;
        Ok(VchanStream { raw: self.raw })
    }
}

pub struct VchanStream {
    raw: RawVchan,
}

impl VchanStream {
    pub fn connect(cfg: &VchanConfig) -> Result<Self> {
        let raw = RawVchan::client(cfg)?;
        Ok(Self { raw })
    }

    pub fn wait(&self) -> Result<()> {
        self.raw.wait()
    }

    pub fn wait_connected(&self) -> Result<()> {
        self.raw.wait_connected()
    }

    pub fn fd_for_select(&self) -> Result<RawFd> {
        self.raw.fd_for_select()
    }

    pub fn is_open(&self) -> Result<i32> {
        self.raw.open_state()
    }

    pub fn data_ready(&self) -> Result<usize> {
        self.raw.data_ready()
    }

    pub fn buffer_space(&self) -> Result<usize> {
        self.raw.buffer_space()
    }

    pub fn read_once(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let rc = unsafe { libxenvchan_read(self.raw.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_read"));
        }
        Ok(rc as usize)
    }

    pub fn read(&mut self, buf: &mut [u8]) -> Result<usize> {
        self.raw.read_some(buf)
    }

    pub fn write_once(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let rc = unsafe { libxenvchan_write(self.raw.as_ptr(), buf.as_ptr().cast(), buf.len()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_write"));
        }
        Ok(rc as usize)
    }

    pub fn write(&mut self, buf: &[u8]) -> Result<usize> {
        self.raw.write_some(buf)
    }

    pub fn recv_once(&mut self, buf: &mut [u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let rc = unsafe { libxenvchan_recv(self.raw.as_ptr(), buf.as_mut_ptr().cast(), buf.len()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_recv"));
        }
        Ok(rc as usize)
    }

    pub fn recv_exact(&mut self, buf: &mut [u8]) -> Result<()> {
        self.raw.recv_exact(buf)
    }

    pub fn send_once(&mut self, buf: &[u8]) -> Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }
        let rc = unsafe { libxenvchan_send(self.raw.as_ptr(), buf.as_ptr().cast(), buf.len()) };
        if rc < 0 {
            return Err(last_error("libxenvchan_send"));
        }
        Ok(rc as usize)
    }

    pub fn send_exact(&mut self, buf: &[u8]) -> Result<()> {
        self.raw.send_exact(buf)
    }
}

fn last_error(op: &'static str) -> Error {
    let source = io::Error::last_os_error();
    let source = match source.raw_os_error() {
        Some(code) if code != 0 => source,
        _ => io::Error::other(op),
    };
    Error::Api { op, source }
}
