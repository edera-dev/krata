use std::{
    ptr::addr_of_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use anyhow::Result;
use libc::{c_int, waitpid, WEXITSTATUS, WIFEXITED};
use log::warn;
use nix::unistd::Pid;
use tokio::sync::broadcast::{channel, Receiver, Sender};

const CHILD_WAIT_QUEUE_LEN: usize = 10;

#[derive(Clone, Copy, Debug)]
pub struct ChildEvent {
    pub pid: Pid,
    pub status: c_int,
}

#[derive(Clone)]
pub struct ChildWait {
    sender: Sender<ChildEvent>,
    signal: Arc<AtomicBool>,
    _task: Arc<JoinHandle<()>>,
}

impl ChildWait {
    pub fn new() -> Result<ChildWait> {
        let (sender, _) = channel(CHILD_WAIT_QUEUE_LEN);
        let signal = Arc::new(AtomicBool::new(false));
        let mut processor = ChildWaitTask {
            sender: sender.clone(),
            signal: signal.clone(),
        };
        let task = thread::spawn(move || {
            if let Err(error) = processor.process() {
                warn!("failed to process child updates: {}", error);
            }
        });
        Ok(ChildWait {
            sender,
            signal,
            _task: Arc::new(task),
        })
    }

    pub async fn subscribe(&self) -> Result<Receiver<ChildEvent>> {
        Ok(self.sender.subscribe())
    }
}

struct ChildWaitTask {
    sender: Sender<ChildEvent>,
    signal: Arc<AtomicBool>,
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

                if self.signal.load(Ordering::Acquire) {
                    return Ok(());
                }
            }
        }
    }
}

impl Drop for ChildWait {
    fn drop(&mut self) {
        if Arc::strong_count(&self.signal) <= 1 {
            self.signal.store(true, Ordering::Release);
        }
    }
}
