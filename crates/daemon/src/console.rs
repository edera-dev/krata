use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, Result};
use circular_buffer::CircularBuffer;
use kratart::channel::ChannelService;
use log::error;
use tokio::{
    sync::{
        mpsc::{error::TrySendError, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};
use uuid::Uuid;

use crate::zlt::ZoneLookupTable;

const CONSOLE_BUFFER_SIZE: usize = 1024 * 1024;
type RawConsoleBuffer = CircularBuffer<CONSOLE_BUFFER_SIZE, u8>;
type ConsoleBuffer = Box<RawConsoleBuffer>;

type ListenerMap = Arc<Mutex<HashMap<u32, Vec<Sender<Vec<u8>>>>>>;
type BufferMap = Arc<Mutex<HashMap<u32, ConsoleBuffer>>>;

#[derive(Clone)]
pub struct DaemonConsoleHandle {
    zlt: ZoneLookupTable,
    listeners: ListenerMap,
    buffers: BufferMap,
    sender: Sender<(u32, Vec<u8>)>,
    task: Arc<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct DaemonConsoleAttachHandle {
    pub initial: Vec<u8>,
    listeners: ListenerMap,
    sender: Sender<(u32, Vec<u8>)>,
    domid: u32,
}

impl DaemonConsoleAttachHandle {
    pub async fn unsubscribe(&self) -> Result<()> {
        let mut guard = self.listeners.lock().await;
        let _ = guard.remove(&self.domid);
        Ok(())
    }

    pub async fn send(&self, data: Vec<u8>) -> Result<()> {
        Ok(self.sender.send((self.domid, data)).await?)
    }
}

impl DaemonConsoleHandle {
    pub async fn attach(
        &self,
        uuid: Uuid,
        sender: Sender<Vec<u8>>,
    ) -> Result<DaemonConsoleAttachHandle> {
        let Some(domid) = self.zlt.lookup_domid_by_uuid(&uuid).await else {
            return Err(anyhow!("unable to find domain {}", uuid));
        };
        let buffers = self.buffers.lock().await;
        let buffer = buffers.get(&domid).map(|x| x.to_vec()).unwrap_or_default();
        drop(buffers);
        let mut listeners = self.listeners.lock().await;
        let senders = listeners.entry(domid).or_default();
        senders.push(sender);
        Ok(DaemonConsoleAttachHandle {
            initial: buffer,
            sender: self.sender.clone(),
            listeners: self.listeners.clone(),
            domid,
        })
    }
}

impl Drop for DaemonConsoleHandle {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}

pub struct DaemonConsole {
    zlt: ZoneLookupTable,
    listeners: ListenerMap,
    buffers: BufferMap,
    receiver: Receiver<(u32, Option<Vec<u8>>)>,
    sender: Sender<(u32, Vec<u8>)>,
    task: JoinHandle<()>,
}

impl DaemonConsole {
    pub async fn new(zlt: ZoneLookupTable) -> Result<DaemonConsole> {
        let (service, sender, receiver) =
            ChannelService::new("krata-console".to_string(), Some(0)).await?;
        let task = service.launch().await?;
        let listeners = Arc::new(Mutex::new(HashMap::new()));
        let buffers = Arc::new(Mutex::new(HashMap::new()));
        Ok(DaemonConsole {
            zlt,
            listeners,
            buffers,
            receiver,
            sender,
            task,
        })
    }

    pub async fn launch(mut self) -> Result<DaemonConsoleHandle> {
        let zlt = self.zlt.clone();
        let listeners = self.listeners.clone();
        let buffers = self.buffers.clone();
        let sender = self.sender.clone();
        let task = tokio::task::spawn(async move {
            if let Err(error) = self.process().await {
                error!("failed to process console: {}", error);
            }
        });
        Ok(DaemonConsoleHandle {
            zlt,
            listeners,
            buffers,
            sender,
            task: Arc::new(task),
        })
    }

    async fn process(&mut self) -> Result<()> {
        loop {
            let Some((domid, data)) = self.receiver.recv().await else {
                break;
            };

            let mut buffers = self.buffers.lock().await;
            if let Some(data) = data {
                let buffer = buffers
                    .entry(domid)
                    .or_insert_with_key(|_| RawConsoleBuffer::boxed());
                buffer.extend_from_slice(&data);
                drop(buffers);
                let mut listeners = self.listeners.lock().await;
                if let Some(senders) = listeners.get_mut(&domid) {
                    senders.retain(|sender| {
                        !matches!(sender.try_send(data.to_vec()), Err(TrySendError::Closed(_)))
                    });
                }
            } else {
                buffers.remove(&domid);
                let mut listeners = self.listeners.lock().await;
                listeners.remove(&domid);
            }
        }
        Ok(())
    }
}

impl Drop for DaemonConsole {
    fn drop(&mut self) {
        self.task.abort();
    }
}
