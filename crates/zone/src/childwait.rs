use anyhow::Result;
use libc::{c_int, waitpid, WEXITSTATUS, WIFEXITED};
use log::warn;
use nix::unistd::Pid;
use std::thread::sleep;
use std::time::Duration;
use std::{
    mem::MaybeUninit,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};
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
    pub fn new() -> Result<(ChildWait, Receiver<ChildEvent>)> {
        let (sender, receiver) = channel(CHILD_WAIT_QUEUE_LEN);
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
        Ok((
            ChildWait {
                sender,
                signal,
                _task: Arc::new(task),
            },
            receiver,
        ))
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
            let mut status = MaybeUninit::<c_int>::new(0);
            let pid = unsafe { waitpid(-1, status.as_mut_ptr(), 0) };
            // pid being -1 indicates an error occurred, wait 100 microseconds to avoid
            // overloading the channel. Right now we don't consider any other errors
            // but that is fine for now, as waitpid shouldn't ever stop anyway.
            if pid == -1 {
                sleep(Duration::from_micros(100));
                continue;
            }
            let status = unsafe { status.assume_init() };
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
