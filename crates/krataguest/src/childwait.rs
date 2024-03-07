use std::{
    ptr::addr_of_mut,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
};

use anyhow::Result;
use log::warn;
use nix::{
    libc::{c_int, wait},
    unistd::Pid,
};
use tokio::sync::mpsc::{channel, Receiver, Sender};

const CHILD_WAIT_QUEUE_LEN: usize = 10;

#[derive(Clone, Copy, Debug)]
pub struct ChildEvent {
    pub pid: Pid,
    pub status: c_int,
}

pub struct ChildWait {
    receiver: Receiver<ChildEvent>,
    signal: Arc<AtomicBool>,
    _task: JoinHandle<()>,
}

impl ChildWait {
    pub fn new() -> Result<ChildWait> {
        let (sender, receiver) = channel(CHILD_WAIT_QUEUE_LEN);
        let signal = Arc::new(AtomicBool::new(false));
        let mut processor = ChildWaitTask {
            sender,
            signal: signal.clone(),
        };
        let task = thread::spawn(move || {
            if let Err(error) = processor.process() {
                warn!("failed to process child updates: {}", error);
            }
        });
        Ok(ChildWait {
            receiver,
            signal,
            _task: task,
        })
    }

    pub async fn recv(&mut self) -> Option<ChildEvent> {
        self.receiver.recv().await
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
            let pid = unsafe { wait(addr_of_mut!(status)) };
            let event = ChildEvent {
                pid: Pid::from_raw(pid),
                status,
            };
            let _ = self.sender.try_send(event);

            if self.signal.load(Ordering::Acquire) {
                return Ok(());
            }
        }
    }
}

impl Drop for ChildWait {
    fn drop(&mut self) {
        self.signal.store(true, Ordering::Release);
    }
}
