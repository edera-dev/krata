use std::{
  collections::HashMap,
  ffi::CString,
  io,
  mem::MaybeUninit,
  path::PathBuf,
  ptr::addr_of_mut,
};

use anyhow::{bail, Context, Result};
use log::{debug, error};

use cgroups_rs::{Cgroup, CgroupPid};
use tokio::sync::broadcast;

use crate::childwait::ChildEvent;

use super::stdio::{StdioSet, Stderr, Stdin, Stdout};

pub struct Child {
  pub stdin:  Option<Stdin>,
  pub stdout: Option<Stdout>,
  pub stderr: Option<Stderr>,
  pid: libc::pid_t,
  reaper_rx: broadcast::Receiver<ChildEvent>,
}

// TODO: impl From<OciImage>
/// Command used to spawn a child process
#[derive(Debug, Clone)]
pub struct ChildSpec {
  /// The executable, with or without path, path relative
  /// or absolute, to run.
  pub exe: PathBuf,
  /// The args to pass, as POSIX specifies
  pub cmd: Vec<CString>,
  /// Env vars to be set
  pub env: HashMap<String, String>,
  /// Working directory to set just before spawning
  pub working_dir: String,
  /// Cgroup we'll use for the child
  pub cgroup: Option<Cgroup>,
  /// Whether to create the child in a new session
  /// This is mainly for image entrypoint
  pub with_new_session: bool,
  /// Whether to use tty
  pub tty: bool,
}

impl Child {
  pub fn pid(&self) -> libc::pid_t { self.pid }

  pub async fn wait(mut self) -> Result<libc::c_int> {
    debug!("waiting on process {}", self.pid);
    loop {
      let Ok(e) = self.reaper_rx.recv().await
        else { bail!("dead reaper - ironic"); };
      
      if e.pid == self.pid {
        return Ok(e.status);
      }
    }
  }
}

impl ChildSpec {
  pub fn spawn(self, reaper_rx: broadcast::Receiver<ChildEvent>) -> Result<Child> {
    let Self {
      exe,
      cmd,
      env,
      working_dir,
      cgroup,
      with_new_session,
      tty,
      ..
    } = self;
   
    let mut stdio = if tty { 
      StdioSet::new_pty().context("failed to spawn pty")?
    } else {
      StdioSet::new_pipes().context("failed to alloc pipes")?
    };

    let mut file_actions: libc::posix_spawn_file_actions_t = unsafe {
      let mut fa = MaybeUninit::uninit();
      libc::posix_spawn_file_actions_init(fa.as_mut_ptr());
      fa.assume_init()
    };
    stdio.add_to_spawn_file_actions(&mut file_actions)?;
   
    let spawnattr: libc::posix_spawnattr_t = unsafe {
      let mut spawnattr = MaybeUninit::uninit();
      libc::posix_spawnattr_init(spawnattr.as_mut_ptr());
      // SAFETY: Both flags use 8 bits or less
      #[allow(overflowing_literals)]
      let mut flags = 0;
      // If we start a new session, spawn will create a new pgroup, too
      if with_new_session {
        flags |= libc::POSIX_SPAWN_SETSID as i16;
      } else {
        flags |= libc::POSIX_SPAWN_SETPGROUP as i16;
      }
 
      match libc::posix_spawnattr_setflags(spawnattr.as_mut_ptr(), flags) {
        x if x > 0 => {
          error!("error on posix_spawnattr_setflags - res {x}");
          return Err(io::Error::last_os_error().into());
        },
        _ => {}
      };

      spawnattr.assume_init()
    };
  
    let old_working_dir = std::env::current_dir().context("failed to retriev CWD")?;
    std::env::set_current_dir(working_dir).context("failed to change CWD")?;
    
    let mut pid: libc::pid_t = 0;
 
    let spawn = if exe.is_relative() {
      debug!("relying on libc to do executable lookup");
      libc::posix_spawnp
    } else {
      debug!("absolute command path found");
      libc::posix_spawn
    };
 
    // SAFETY: We're using the raw underlying value, then rewrapping it for Drop
    let res = unsafe {
      let exe = CString::new(exe.as_os_str().as_encoded_bytes())?;
      let cmd = cmd.into_iter()
        .map(CString::into_raw)
        .chain(Some(std::ptr::null_mut()))
        .collect::<Vec<*mut i8>>();

      let env = env.iter()
        .map(|(key, value)| CString::new(format!("{}={}", key, value)).context("null byte in env vars"))
        .collect::<Result<Vec<CString>>>()?;
      let env = env.into_iter()
        .map(CString::into_raw)
        .chain(Some(std::ptr::null_mut()))
        .collect::<Vec<*mut i8>>();
 
      // TODO: Safety comment
      let res = spawn(
        addr_of_mut!(pid),
        exe.as_ptr(),
        &file_actions,
        &spawnattr,
        cmd.as_slice().as_ptr(),
        env.as_slice().as_ptr(),
      );
 
      let _ = cmd.into_iter().map(|a| CString::from_raw(a));
      let _ = env.into_iter().map(|e| CString::from_raw(e));
      
      res
    };
 
    std::env::set_current_dir(old_working_dir).context("failed to restore previous CWD")?;
 
    if res != 0 {
      error!("Failed to spawn process: return value of {res}");
      return Err(io::Error::last_os_error().into());
    }
   
    if let Some(cg) = cgroup {
      cg.add_task(CgroupPid::from(pid as u64)).context("failed to add child to cgroup")?;
    }

    let (stdin, stdout, stderr) = stdio.get_parent_side()?;
 
    Ok(Child {
      pid,
      reaper_rx,
      stdin:  Some(stdin),
      stdout: Some(stdout),
      stderr: Some(stderr),
    })
  }
}

