pub mod error;
pub mod raw;
pub mod sys;

use crate::error::{Error, Result};
use crate::sys::{
    BindInterdomainRequest, BindUnboundPortRequest, BindVirqRequest, NotifyRequest,
    UnbindPortRequest,
};

use crate::raw::EVENT_CHANNEL_DEVICE;
use byteorder::{LittleEndian, ReadBytesExt};
use log::error;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::mem::size_of;
use std::os::fd::AsRawFd;
use std::os::raw::c_void;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::fs::{File, OpenOptions};
use tokio::sync::{Mutex, Notify};

type WakeMap = Arc<Mutex<HashMap<u32, Arc<Notify>>>>;

#[derive(Clone)]
pub struct EventChannelService {
    handle: Arc<Mutex<File>>,
    wakes: WakeMap,
    process_flag: Arc<AtomicBool>,
}

pub struct BoundEventChannel {
    pub local_port: u32,
    pub receiver: Arc<Notify>,
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
        let wakes = Arc::new(Mutex::new(HashMap::new()));
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
        let fd = handle.as_raw_fd();
        let mut request = BindVirqRequest { virq };
        let result =
            tokio::task::spawn_blocking(move || unsafe { sys::bind_virq(fd, &mut request) })
                .await
                .map_err(|_| Error::BlockingTaskJoin)?? as u32;
        Ok(result)
    }

    pub async fn bind_interdomain(&self, domid: u32, port: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        let fd = handle.as_raw_fd();
        let mut request = BindInterdomainRequest {
            remote_domain: domid,
            remote_port: port,
        };
        let result =
            tokio::task::spawn_blocking(move || unsafe { sys::bind_interdomain(fd, &mut request) })
                .await
                .map_err(|_| Error::BlockingTaskJoin)?? as u32;
        Ok(result)
    }

    pub async fn bind_unbound_port(&self, domid: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        let fd = handle.as_raw_fd();
        let mut request = BindUnboundPortRequest {
            remote_domain: domid,
        };
        let result = tokio::task::spawn_blocking(move || unsafe {
            sys::bind_unbound_port(fd, &mut request)
        })
        .await
        .map_err(|_| Error::BlockingTaskJoin)?? as u32;
        Ok(result)
    }

    pub async fn unmask(&self, port: u32) -> Result<()> {
        let handle = self.handle.lock().await;
        let mut port = port;
        let fd = handle.as_raw_fd();
        let result = tokio::task::spawn_blocking(move || unsafe {
            libc::write(fd, &mut port as *mut u32 as *mut c_void, size_of::<u32>())
        })
        .await
        .map_err(|_| Error::BlockingTaskJoin)?;
        if result != size_of::<u32>() as isize {
            return Err(Error::Io(std::io::Error::from_raw_os_error(result as i32)));
        }
        Ok(())
    }

    pub async fn unbind(&self, port: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        let mut request = UnbindPortRequest { port };
        let fd = handle.as_raw_fd();
        let result = tokio::task::spawn_blocking(move || unsafe { sys::unbind(fd, &mut request) })
            .await
            .map_err(|_| Error::BlockingTaskJoin)?? as u32;
        self.wakes.lock().await.remove(&port);
        Ok(result)
    }

    pub async fn notify(&self, port: u32) -> Result<u32> {
        let handle = self.handle.lock().await;
        let mut request = NotifyRequest { port };
        let fd = handle.as_raw_fd();
        let result = tokio::task::spawn_blocking(move || unsafe { sys::notify(fd, &mut request) })
            .await
            .map_err(|_| Error::BlockingTaskJoin)?? as u32;
        Ok(result)
    }

    pub async fn reset(&self) -> Result<u32> {
        let handle = self.handle.lock().await;
        let fd = handle.as_raw_fd();
        let result = tokio::task::spawn_blocking(move || unsafe { sys::reset(fd) })
            .await
            .map_err(|_| Error::BlockingTaskJoin)?? as u32;
        Ok(result)
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

    pub async fn subscribe(&self, port: u32) -> Result<Arc<Notify>> {
        let mut wakes = self.wakes.lock().await;
        let receiver = match wakes.entry(port) {
            Entry::Occupied(entry) => entry.get().clone(),

            Entry::Vacant(entry) => {
                let notify = Arc::new(Notify::new());
                entry.insert(notify.clone());
                notify
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
                error!("failed to process event channel wakes: {}", error);
            }
        });

        Ok(())
    }

    pub fn process(&mut self) -> Result<()> {
        loop {
            let port = self.handle.read_u32::<LittleEndian>()?;
            let receiver = match self.wakes.blocking_lock().entry(port) {
                Entry::Occupied(entry) => entry.get().clone(),

                Entry::Vacant(entry) => {
                    let notify = Arc::new(Notify::new());
                    entry.insert(notify.clone());
                    notify
                }
            };
            receiver.notify_one();
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
