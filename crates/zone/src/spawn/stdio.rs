use std::{
  io,
  os::fd::{AsRawFd, IntoRawFd, RawFd},
  pin::Pin,
  task::{Context, Poll, ready},
};

use anyhow::{bail, Context as _, Result};

use pin_project_lite::pin_project;
use tokio::io::{
  AsyncRead, AsyncWrite, Interest, ReadBuf,
  unix::AsyncFd,
};

type SpawnFileActions = libc::posix_spawn_file_actions_t;

pub struct StdioSet {
  parent: Option<StdioSubset>,
  child:  Option<StdioSubset>,
}

pub struct StdioSubset {
  stdin:  Stdio,
  stdout: Stdio,
  stderr: Stdio,
}

pin_project! {
  pub struct Stdin {
    #[pin] inner: Stdio
  }
}

pin_project! {
  pub struct Stdout {
    #[pin] inner: Stdio
  }
}

pin_project! {
  pub struct Stderr {
    #[pin] inner: Stdio
  }
}

struct Stdio(RawFd);

impl StdioSet {
  pub fn add_to_spawn_file_actions(&mut self, attr: &mut SpawnFileActions) -> Result<()> {
    let Some(stdio) = self.child.take() else { bail!("already used child-side fd's") };
    let res_in = unsafe {
      libc::posix_spawn_file_actions_adddup2(
        attr, stdio.stdin.0, libc::STDIN_FILENO
      )
    };
 
    let res_out = unsafe {
      libc::posix_spawn_file_actions_adddup2(
        attr, stdio.stdout.0, libc::STDOUT_FILENO
      )
    };
 
    let res_err = unsafe { 
      libc::posix_spawn_file_actions_adddup2(
        attr, stdio.stderr.0, libc::STDERR_FILENO
      )
    };

    // It is highly unlikely that they will fail from different errors, and
    // even if they did, they're all fatal and need to be addressed by the 
    // user deploying.
    match (res_in, res_out, res_err) {
      (0, 0, 0) => Ok(()),
      _ => Err(std::io::Error::last_os_error().into()),
    }
  }

  pub fn get_parent_side(&mut self) -> Result<(Stdin, Stdout, Stderr)> {
    let StdioSubset { stdin, stdout, stderr }
      = self.parent.take().context("stdio handles already taken")?;

    Ok((Stdin { inner: stdin }, Stdout { inner: stdout }, Stderr { inner: stderr }))
  }

  pub fn new_pty() -> Result<Self> {
    use nix::{fcntl::{self, FcntlArg, OFlag}, pty};

    // Open the Pseudoterminal with +rw capabilities and without
    // setting it as our controlling terminal
    let pty = pty::posix_openpt(OFlag::O_RDWR | OFlag::O_NOCTTY)?;
    // Grant access to the side we pass to the child
    // This is referred to as the "slave"
    pty::grantpt(&pty)?;
    // Unlock the "slave" device
    pty::unlockpt(&pty)?;

    // Retrieve the "slave" device
    let pts = {
      let name = pty::ptsname_r(&pty)?;
      std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(name)?
        .into_raw_fd()
    };

    // Get the RawFd out of the OwnedFd because OwnedFd
    // sets CLOEXEC on clone
    let pty = pty.as_raw_fd();

    // Make the "master" async-ready by setting NONBLOCK
    let mut opts = OFlag::from_bits(fcntl::fcntl(pty, FcntlArg::F_GETFL)?)
      .expect("got bad O_FLAG bits from kernel");
    opts |= OFlag::O_NONBLOCK;
    fcntl::fcntl(pty, FcntlArg::F_SETFL(opts))?;

    Ok(Self {
      child: Some(StdioSubset {
        stdin:  Stdio(pts.clone()),
        stdout: Stdio(pts.clone()),
        stderr: Stdio(pts),
      }),
      parent: Some(StdioSubset {
        stdin:  Stdio(pty.clone()),
        stdout: Stdio(pty.clone()),
        stderr: Stdio(pty),
      }),
    })
  }

  pub fn new_pipes() -> Result<Self> {
    let (stdin_child,   stdin_parent) = make_pipe()?;
    let (stdout_parent, stdout_child) = make_pipe()?;
    let (stderr_parent, stderr_child) = make_pipe()?;

    Ok(Self {
      parent: Some(StdioSubset {
        stdin:  Stdio(stdin_parent),
        stdout: Stdio(stdout_parent),
        stderr: Stdio(stderr_parent),
      }),
      child: Some(StdioSubset {
        stdin:  Stdio(stdin_child),
        stdout: Stdio(stdout_child),
        stderr: Stdio(stderr_child),
      }),
    })
  }
}

impl AsyncRead for Stdio {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>
  ) -> Poll<std::io::Result<()>> {
    // SAFETY: if this fails, we have a bug in our pty/pipe allocations
    let fd = AsyncFd::with_interest(self.0, Interest::READABLE)
      .expect("async io failure");

    loop {
      let mut guard = ready!(fd.poll_read_ready(cx))?;
      let count = buf.remaining();

      let res = guard.try_io(|i| match unsafe {
        let buf_ptr = buf.initialize_unfilled().as_mut_ptr().cast();
        libc::read(i.as_raw_fd(), buf_ptr, count)
      } {
        -1 => Err(std::io::Error::last_os_error()),
        // SAFETY: write returns -1..=isize::MAX, and
        // we've already ruled out -1, so this will be
        // a valid usize.
        n => { buf.advance(n.try_into().unwrap()); Ok(()) }
      });
        
      if let Ok(r) = res {
        // Err will ever only be WouldBlock, so we allow
        // the loop to try again. `r` is the inner Result
        // of try_io
        return Poll::Ready(r);
      }
    }
  }
}

impl AsyncWrite for Stdio {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8]
  ) -> Poll<io::Result<usize>> {
    // SAFETY: if this fails, we have a bug in our pty/pipe allocations
    let fd = AsyncFd::with_interest(self.0, Interest::WRITABLE)
      .expect("async io failure");

    loop {
      let mut guard = ready!(fd.poll_write_ready(cx))?;

      let res = guard.try_io(|i| match unsafe {
        libc::write(i.as_raw_fd(), buf.as_ptr().cast(), buf.len())
      } {
        -1 => Err(io::Error::last_os_error()),
        // SAFETY: write returns -1..=isize::MAX, and
        // we've already ruled out -1, so this will be
        // a valid usize.
        n => Ok(n.try_into().unwrap()),
      });

      if let Ok(r) = res {
        // Err will ever only be WouldBlock, so we allow
        // the loop to try again. `r` is the inner Result
        // of try_io
        return Poll::Ready(r);
      }
    }
  }

  fn poll_flush(
    self: Pin<&mut Self>,
    _cx: &mut Context<'_>
  ) -> Poll<io::Result<()>> {
    Poll::Ready(Ok(()))
  }

  fn poll_shutdown(
    self: Pin<&mut Self>,
    _cx: &mut Context<'_>
  ) -> Poll<io::Result<()>> {
    Poll::Ready(Ok(()))
  }
}

impl AsyncWrite for Stdin {
  fn poll_write(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &[u8]
  ) -> Poll<io::Result<usize>> {
    self.project().inner.poll_write(cx, buf)
  }

  fn poll_flush(
    self: Pin<&mut Self>,
    _cx: &mut Context<'_>
  ) -> Poll<io::Result<()>> {
    Poll::Ready(Ok(()))
  }

  fn poll_shutdown(
    self: Pin<&mut Self>,
    _cx: &mut Context<'_>
  ) -> Poll<io::Result<()>> {
    Poll::Ready(Ok(()))
  }
}

impl AsyncRead for Stdout {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>
  ) -> Poll<io::Result<()>> {
    self.project().inner.poll_read(cx, buf)
  }
}

impl AsyncRead for Stderr {
  fn poll_read(
    self: Pin<&mut Self>,
    cx: &mut Context<'_>,
    buf: &mut ReadBuf<'_>
  ) -> Poll<io::Result<()>> {
    self.project().inner.poll_read(cx, buf)
  }
}

fn make_pipe() -> Result<(RawFd, RawFd)> {
  // Init two null file descriptors
  // [read, write]
  let mut raw_fds: [RawFd; 2] = [0, 0];

  // Allocate the pipe and get each end of, setting as non-blocking
  let res = unsafe { libc::pipe(raw_fds.as_mut_ptr().cast()) };
  if res == -1 { return Err(io::Error::last_os_error().into()); }

  // We split the pipe into its ends so we can be explicit
  // which end is which.
  let [read, write] = raw_fds;

  // Wipe the flags, because CLOEXEC is on by default
  let flags = libc::O_NONBLOCK;
  f_setfl(read, flags)?;
  f_setfl(write, flags)?;

  Ok((read, write))
}

fn f_setfl(fd: RawFd, flags: libc::c_int) -> Result<()> {
  let res = unsafe { libc::fcntl(fd, libc::F_SETFL, flags) };
  if res == -1 { return Err(io::Error::last_os_error().into()); }

  Ok(())
}
