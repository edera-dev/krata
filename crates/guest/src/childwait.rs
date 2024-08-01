use std::{
    ptr::addr_of_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use anyhow::Result;
use log::warn;
use libc::{c_int, waitpid, WEXITSTATUS, WIFEXITED};

use nix::unistd::Pid;
use tokio::{
  sync::broadcast::{channel, Receiver, Sender},
  task::{self, JoinHandle},
};

const CHILD_WAIT_QUEUE_LEN: usize = 10;

#[derive(Clone, Copy, Debug)]
pub struct ChildEvent {
  pub pid: Pid,
  pub status: c_int,
}

pub struct ChildWait {
  pub receiver: Receiver<ChildEvent>,
  _task: JoinHandle<()>,
}

impl ChildWait {
  pub fn new() -> Result<ChildWait> {
    let (sender, receiver) = channel(CHILD_WAIT_QUEUE_LEN);
    let mut processor = ChildWaitTask {
      sender,
    };
    let task = task::spawn_blocking(move || {
      if let Err(error) = processor.process() {
        warn!("failed to process child updates: {}", error);
      }
    });
    Ok(ChildWait {
      receiver,
      _task: task,
    })
  }
}

struct ChildWaitTask {
  sender: Sender<ChildEvent>,
}

impl ChildWaitTask {
  fn process(&mut self) -> Result<()> {
    loop {
      let mut status: c_int = 0;
      let pid = unsafe { waitpid(-1, addr_of_mut!(status), 0) };

      if WIFEXITED(status) {
        let event = ChildEvent {
          pid: Pid::from_raw(pid),
          status: WEXITSTATUS(status),
        };
        let _ = self.sender.send(event);
      }
    }
  }
}

