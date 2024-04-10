use std::{collections::HashMap, sync::Arc};

use anyhow::Result;
use bytes::{Buf, BytesMut};
use krata::idm::protocol::IdmPacket;
use kratart::channel::ChannelService;
use log::{error, warn};
use prost::Message;
use tokio::{
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

type ListenerMap = Arc<Mutex<HashMap<u32, Sender<(u32, IdmPacket)>>>>;

#[derive(Clone)]
pub struct DaemonIdmHandle {
    listeners: ListenerMap,
    tx_sender: Sender<(u32, IdmPacket)>,
    task: Arc<JoinHandle<()>>,
}

#[derive(Clone)]
pub struct DaemonIdmSubscribeHandle {
    domid: u32,
    tx_sender: Sender<(u32, IdmPacket)>,
    listeners: ListenerMap,
}

impl DaemonIdmSubscribeHandle {
    pub async fn send(&self, packet: IdmPacket) -> Result<()> {
        self.tx_sender.send((self.domid, packet)).await?;
        Ok(())
    }

    pub async fn unsubscribe(&self) -> Result<()> {
        let mut guard = self.listeners.lock().await;
        let _ = guard.remove(&self.domid);
        Ok(())
    }
}

impl DaemonIdmHandle {
    pub async fn send(&self, domid: u32, packet: IdmPacket) -> Result<()> {
        self.tx_sender.send((domid, packet)).await?;
        Ok(())
    }

    pub async fn subscribe(
        &self,
        domid: u32,
        sender: Sender<(u32, IdmPacket)>,
    ) -> Result<DaemonIdmSubscribeHandle> {
        let mut guard = self.listeners.lock().await;
        guard.insert(domid, sender);
        Ok(DaemonIdmSubscribeHandle {
            domid,
            tx_sender: self.tx_sender.clone(),
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
    tx_sender: Sender<(u32, IdmPacket)>,
    tx_raw_sender: Sender<(u32, Vec<u8>)>,
    tx_receiver: Receiver<(u32, IdmPacket)>,
    rx_receiver: Receiver<(u32, Vec<u8>)>,
    task: JoinHandle<()>,
}

impl DaemonIdm {
    pub async fn new() -> Result<DaemonIdm> {
        let (service, tx_raw_sender, rx_receiver) =
            ChannelService::new("krata-channel".to_string(), None).await?;
        let (tx_sender, tx_receiver) = channel(100);
        let task = service.launch().await?;
        let listeners = Arc::new(Mutex::new(HashMap::new()));
        Ok(DaemonIdm {
            rx_receiver,
            tx_receiver,
            tx_sender,
            tx_raw_sender,
            task,
            listeners,
        })
    }

    pub async fn launch(mut self) -> Result<DaemonIdmHandle> {
        let listeners = self.listeners.clone();
        let tx_sender = self.tx_sender.clone();
        let task = tokio::task::spawn(async move {
            let mut buffers: HashMap<u32, BytesMut> = HashMap::new();
            if let Err(error) = self.process(&mut buffers).await {
                error!("failed to process idm: {}", error);
            }
        });
        Ok(DaemonIdmHandle {
            listeners,
            tx_sender,
            task: Arc::new(task),
        })
    }

    async fn process(&mut self, buffers: &mut HashMap<u32, BytesMut>) -> Result<()> {
        loop {
            select! {
                x = self.rx_receiver.recv() => match x {
                    Some((domid, data)) => {
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
                    },

                    None => {
                        break;
                    }
                },
                x = self.tx_receiver.recv() => match x {
                    Some((domid, packet)) => {
                        let data = packet.encode_to_vec();
                        self.tx_raw_sender.send((domid, data)).await?;
                    },

                    None => {
                        break;
                    }
                }
            };
        }
        Ok(())
    }
}

impl Drop for DaemonIdm {
    fn drop(&mut self) {
        self.task.abort();
    }
}
