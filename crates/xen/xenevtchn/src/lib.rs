pub mod error;
pub mod raw;
pub mod sys;

use crate::error::{Error, Result};
use crate::sys::{BindInterdomain, BindUnboundPort, BindVirq, Notify, UnbindPort};

use crate::raw::EVENT_CHANNEL_DEVICE;
use byteorder::{LittleEndian, ReadBytesExt};
use log::warn;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::sync::{Mutex, RwLock};

const CHANNEL_QUEUE_LEN: usize = 30;

type WakeMap = Arc<RwLock<HashMap<u32, Sender<u32>>>>;

#[derive(Clone)]
pub struct EventChannelService {
    handle: Arc<Mutex<File>>,
    wakes: WakeMap,
    process_flag: Arc<AtomicBool>,
}

pub struct BoundEventChannel {
    pub local_port: u32,
    pub receiver: Receiver<u32>,
    pub service: EventChannelService,
}

impl BoundEventChannel {
    pub async fn unmask(&self) -> Result<()> {
        self.service.unmask(self.local_port).await
    }
}

impl Drop for BoundEventChannel {
    fn drop(&mut self) {
        let service = self.service.clone();
        let port = self.local_port;
        tokio::task::spawn(async move {
            let _ = service.unbind(port).await;
        });
    }
}

impl EventChannelService {
    pub async fn open() -> Result<EventChannelService> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open(EVENT_CHANNEL_DEVICE)
            .await?;
        let wakes = Arc::new(RwLock::new(HashMap::new()));
        let flag = Arc::new(AtomicBool::new(false));
        let processor = EventChannelProcessor {
            flag: flag.clone(),
            handle: handle.try_clone().await?.into_std().await,
            wakes: wakes.clone(),
        };
        processor.launch()?;

        Ok(EventChannelService {
            handle: Arc::new(Mutex::new(handle)),
            wakes,
            process_flag: flag,
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

    pub async fn unmask(&self, port: u32) -> Result<()> {
        let handle = self.handle.lock().await;
        let mut port = port;
        let result = unsafe {
            libc::write(
                handle.as_raw_fd(),
                &mut port as *mut u32 as *mut c_void,
                size_of::<u32>(),
            )
        };
        if result != size_of::<u32>() as isize {
            return Err(Error::Io(std::io::Error::from_raw_os_error(result as i32)));
        }
        Ok(())
    }

    pub async fn unbind(&self, port: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        unsafe {
            let mut request = UnbindPort { port };
            let result = sys::unbind(handle.as_raw_fd(), &mut request)? as u32;
            self.wakes.write().await.remove(&port);
            Ok(result)
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
        let receiver = self.subscribe(local_port).await?;
        let bound = BoundEventChannel {
            local_port,
            receiver,
            service: self.clone(),
        };
        Ok(bound)
    }

    pub async fn subscribe(&self, port: u32) -> Result<Receiver<u32>> {
        let mut wakes = self.wakes.write().await;
        let receiver = match wakes.entry(port) {
            Entry::Occupied(_) => {
                return Err(Error::PortInUse);
            }

            Entry::Vacant(entry) => {
                let (sender, receiver) = channel::<u32>(CHANNEL_QUEUE_LEN);
                entry.insert(sender);
                receiver
            }
        };
        Ok(receiver)
    }
}

pub struct EventChannelProcessor {
    flag: Arc<AtomicBool>,
    handle: std::fs::File,
    wakes: WakeMap,
}

impl EventChannelProcessor {
    pub fn launch(mut self) -> Result<()> {
        std::thread::spawn(move || {
            while let Err(error) = self.process() {
                if self.flag.load(Ordering::Acquire) {
                    break;
                }
                warn!("failed to process event channel notifications: {}", error);
            }
        });
        Ok(())
    }

    pub fn process(&mut self) -> Result<()> {
        loop {
            let port = self.handle.read_u32::<LittleEndian>()?;
            if let Some(wake) = self.wakes.blocking_read().get(&port) {
                let _ = wake.try_send(port);
            }
        }
    }
}

impl Drop for EventChannelService {
    fn drop(&mut self) {
        if Arc::strong_count(&self.handle) <= 1 {
            self.process_flag.store(true, Ordering::Release);
        }
    }
}
