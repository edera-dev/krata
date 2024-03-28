use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use bytes::{Buf, BytesMut};
use krata::idm::protocol::IdmPacket;
use kratart::channel::ChannelService;
use log::{error, warn};
use prost::Message;
use tokio::{
    sync::{
        mpsc::{Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

type ListenerMap = Arc<Mutex<HashMap<u32, Sender<(u32, IdmPacket)>>>>;

#[derive(Clone)]
pub struct DaemonIdmHandle {
    listeners: ListenerMap,
    task: Arc<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct DaemonIdmSubscribeHandle {
    domid: u32,
    listeners: ListenerMap,
}

impl DaemonIdmSubscribeHandle {
    pub async fn unsubscribe(&self) -> Result<()> {
        let mut guard = self.listeners.lock().await;
        let _ = guard.remove(&self.domid);
        Ok(())
    }
}

impl DaemonIdmHandle {
    pub async fn subscribe(
        &self,
        domid: u32,
        sender: Sender<(u32, IdmPacket)>,
    ) -> Result<DaemonIdmSubscribeHandle> {
        let mut guard = self.listeners.lock().await;
        guard.insert(domid, sender);
        Ok(DaemonIdmSubscribeHandle {
            domid,
            listeners: self.listeners.clone(),
        })
    }
}

impl Drop for DaemonIdmHandle {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}

pub struct DaemonIdm {
    listeners: ListenerMap,
    receiver: Receiver<(u32, Vec<u8>)>,
    task: JoinHandle<()>,
}

impl DaemonIdm {
    pub async fn new() -> Result<DaemonIdm> {
        let (service, receiver) = ChannelService::new("krata-channel".to_string()).await?;
        let task = service.launch().await?;
        let listeners = Arc::new(Mutex::new(HashMap::new()));
        Ok(DaemonIdm {
            receiver,
            task,
            listeners,
        })
    }

    pub async fn launch(mut self) -> Result<DaemonIdmHandle> {
        let listeners = self.listeners.clone();
        let task = tokio::task::spawn(async move {
            let mut buffers: HashMap<u32, BytesMut> = HashMap::new();
            if let Err(error) = self.process(&mut buffers).await {
                error!("failed to process idm: {}", error);
            }
        });
        Ok(DaemonIdmHandle {
            listeners,
            task: Arc::new(task),
        })
    }

    async fn process(&mut self, buffers: &mut HashMap<u32, BytesMut>) -> Result<()> {
        loop {
            let Some((domid, data)) = self.receiver.recv().await else {
                break;
            };

            let buffer = buffers.entry(domid).or_insert_with_key(|_| BytesMut::new());
            buffer.extend_from_slice(&data);
            if buffer.len() < 2 {
                continue;
            }
            let size = (buffer[0] as u16 | (buffer[1] as u16) << 8) as usize;
            let needed = size + 2;
            if buffer.len() < needed {
                continue;
            }
            let mut packet = buffer.split_to(needed);
            packet.advance(2);
            match IdmPacket::decode(packet) {
                Ok(packet) => {
                    let guard = self.listeners.lock().await;
                    if let Some(sender) = guard.get(&domid) {
                        if let Err(error) = sender.try_send((domid, packet)) {
                            warn!("dropped idm packet from domain {}: {}", domid, error);
                        }
                    }
                }

                Err(packet) => {
                    warn!("received invalid packet from domain {}: {}", domid, packet);
                }
            }
        }
        Ok(())
    }
}

impl Drop for DaemonIdm {
    fn drop(&mut self) {
        self.task.abort();
    }
}
