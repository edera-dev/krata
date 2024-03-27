pub mod error;
pub mod sys;

use crate::error::{Error, Result};
use crate::sys::{BindInterdomain, BindUnboundPort, BindVirq, Notify, UnbindPort};

use log::error;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::raw::c_void;
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncReadExt;
use tokio::select;
use tokio::sync::broadcast::{
    channel as broadcast_channel, Receiver as BroadcastReceiver, Sender as BroadastSender,
};
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::Mutex;
use tokio::task::JoinHandle;

const UNBIND_CHANNEL_QUEUE_LEN: usize = 30;
const UNMASK_CHANNEL_QUEUE_LEN: usize = 30;
const BROADCAST_CHANNEL_QUEUE_LEN: usize = 30;

type WakeMap = Arc<Mutex<HashMap<u32, BroadastSender<u32>>>>;

#[derive(Clone)]
pub struct EventChannel {
    handle: Arc<Mutex<File>>,
    wakes: WakeMap,
    unbind_sender: Sender<u32>,
    unmask_sender: Sender<u32>,
    task: Arc<JoinHandle<()>>,
}

pub struct BoundEventChannel {
    pub local_port: u32,
    pub receiver: BroadcastReceiver<u32>,
    unbind_sender: Sender<u32>,
    pub unmask_sender: Sender<u32>,
}

impl Drop for BoundEventChannel {
    fn drop(&mut self) {
        let _ = self.unbind_sender.try_send(self.local_port);
    }
}

impl EventChannel {
    pub async fn open() -> Result<EventChannel> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/evtchn")
            .await?;

        let wakes = Arc::new(Mutex::new(HashMap::new()));
        let (unbind_sender, unbind_receiver) = channel(UNBIND_CHANNEL_QUEUE_LEN);
        let (unmask_sender, unmask_receiver) = channel(UNMASK_CHANNEL_QUEUE_LEN);
        let task = {
            let file = file.try_clone().await?;
            let wakes = wakes.clone();
            tokio::task::spawn(async move {
                if let Err(error) =
                    EventChannel::process(file, wakes, unmask_receiver, unbind_receiver).await
                {
                    error!("event channel processor failed: {}", error);
                }
            })
        };
        Ok(EventChannel {
            handle: Arc::new(Mutex::new(file)),
            wakes,
            unbind_sender,
            unmask_sender,
            task: Arc::new(task),
        })
    }

    pub async fn bind_virq(&self, virq: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        unsafe {
            let mut request = BindVirq { virq };
            Ok(sys::bind_virq(handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub async fn bind_interdomain(&self, domid: u32, port: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        unsafe {
            let mut request = BindInterdomain {
                remote_domain: domid,
                remote_port: port,
            };
            Ok(sys::bind_interdomain(handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub async fn bind_unbound_port(&self, domid: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        unsafe {
            let mut request = BindUnboundPort {
                remote_domain: domid,
            };
            Ok(sys::bind_unbound_port(handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub async fn unbind(&self, port: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        unsafe {
            let mut request = UnbindPort { port };
            Ok(sys::unbind(handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub async fn notify(&self, port: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        unsafe {
            let mut request = Notify { port };
            Ok(sys::notify(handle.as_raw_fd(), &mut request)? as u32)
        }
    }

    pub async fn reset(&self) -> Result<u32> {
        let handle = self.handle.lock().await;
        unsafe { Ok(sys::reset(handle.as_raw_fd())? as u32) }
    }

    pub async fn bind(&self, domid: u32, port: u32) -> Result<BoundEventChannel> {
        let local_port = self.bind_interdomain(domid, port).await?;
        let (receiver, unmask_sender) = self.subscribe(local_port).await?;
        let bound = BoundEventChannel {
            local_port,
            receiver,
            unbind_sender: self.unbind_sender.clone(),
            unmask_sender,
        };
        Ok(bound)
    }

    pub async fn subscribe(&self, port: u32) -> Result<(BroadcastReceiver<u32>, Sender<u32>)> {
        let mut wakes = self.wakes.lock().await;
        let receiver = match wakes.entry(port) {
            Entry::Occupied(entry) => entry.get().subscribe(),

            Entry::Vacant(entry) => {
                let (sender, receiver) = broadcast_channel::<u32>(BROADCAST_CHANNEL_QUEUE_LEN);
                entry.insert(sender);
                receiver
            }
        };
        Ok((receiver, self.unmask_sender.clone()))
    }

    async fn process(
        mut file: File,
        wakers: WakeMap,
        mut unmask_receiver: Receiver<u32>,
        mut unbind_receiver: Receiver<u32>,
    ) -> Result<()> {
        loop {
            select! {
                result = file.read_u32_le() => {
                    match result {
                        Ok(port) => {
                            if let Some(sender) = wakers.lock().await.get(&port) {
                                if let Err(error) = sender.send(port) {
                                    return Err(Error::WakeSend(error));
                                }
                            }
                        }

                        Err(error) => return Err(Error::Io(error))
                    }
                }

                result = unmask_receiver.recv() => {
                    match result {
                        Some(port) => {
                            unsafe {
                                let mut port = port;
                                let result = libc::write(file.as_raw_fd(), &mut port as *mut u32 as *mut c_void, size_of::<u32>());
                                if result != size_of::<u32>() as isize {
                                    return Err(Error::Io(std::io::Error::from_raw_os_error(result as i32)));
                                }
                            }
                        }

                        None => {
                            break;
                        }
                    }
                }

                result = unbind_receiver.recv() => {
                    match result {
                        Some(port) => {
                            unsafe {
                                let mut request = UnbindPort { port };
                                sys::unbind(file.as_raw_fd(), &mut request)?;
                            }
                        }

                        None => {
                            break;
                        }
                    }
                }
            };
        }

        Ok(())
    }
}

impl Drop for EventChannel {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}
