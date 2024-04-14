use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use anyhow::{anyhow, Result};
use bytes::{Buf, BytesMut};
use krata::idm::{
    client::{IdmBackend, IdmClient},
    protocol::IdmPacket,
};
use kratart::channel::ChannelService;
use log::{error, warn};
use prost::Message;
use tokio::{
    select,
    sync::{
        broadcast,
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

type BackendFeedMap = Arc<Mutex<HashMap<u32, Sender<IdmPacket>>>>;
type ClientMap = Arc<Mutex<HashMap<u32, IdmClient>>>;

#[derive(Clone)]
pub struct DaemonIdmHandle {
    clients: ClientMap,
    feeds: BackendFeedMap,
    tx_sender: Sender<(u32, IdmPacket)>,
    task: Arc<JoinHandle<()>>,
    snoop_sender: broadcast::Sender<DaemonIdmSnoopPacket>,
}

impl DaemonIdmHandle {
    pub fn snoop(&self) -> broadcast::Receiver<DaemonIdmSnoopPacket> {
        self.snoop_sender.subscribe()
    }

    pub async fn client(&self, domid: u32) -> Result<IdmClient> {
        client_or_create(domid, &self.tx_sender, &self.clients, &self.feeds).await
    }
}

impl Drop for DaemonIdmHandle {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}

#[derive(Clone)]
pub struct DaemonIdmSnoopPacket {
    pub from: u32,
    pub to: u32,
    pub packet: IdmPacket,
}

pub struct DaemonIdm {
    clients: ClientMap,
    feeds: BackendFeedMap,
    tx_sender: Sender<(u32, IdmPacket)>,
    tx_raw_sender: Sender<(u32, Vec<u8>)>,
    tx_receiver: Receiver<(u32, IdmPacket)>,
    rx_receiver: Receiver<(u32, Option<Vec<u8>>)>,
    snoop_sender: broadcast::Sender<DaemonIdmSnoopPacket>,
    task: JoinHandle<()>,
}

impl DaemonIdm {
    pub async fn new() -> Result<DaemonIdm> {
        let (service, tx_raw_sender, rx_receiver) =
            ChannelService::new("krata-channel".to_string(), None).await?;
        let (tx_sender, tx_receiver) = channel(100);
        let (snoop_sender, _) = broadcast::channel(100);
        let task = service.launch().await?;
        let clients = Arc::new(Mutex::new(HashMap::new()));
        let feeds = Arc::new(Mutex::new(HashMap::new()));
        Ok(DaemonIdm {
            rx_receiver,
            tx_receiver,
            tx_sender,
            tx_raw_sender,
            snoop_sender,
            task,
            clients,
            feeds,
        })
    }

    pub async fn launch(mut self) -> Result<DaemonIdmHandle> {
        let clients = self.clients.clone();
        let feeds = self.feeds.clone();
        let tx_sender = self.tx_sender.clone();
        let snoop_sender = self.snoop_sender.clone();
        let task = tokio::task::spawn(async move {
            let mut buffers: HashMap<u32, BytesMut> = HashMap::new();

            while let Err(error) = self.process(&mut buffers).await {
                error!("failed to process idm: {}", error);
            }
        });
        Ok(DaemonIdmHandle {
            clients,
            feeds,
            tx_sender,
            snoop_sender,
            task: Arc::new(task),
        })
    }

    async fn process(&mut self, buffers: &mut HashMap<u32, BytesMut>) -> Result<()> {
        loop {
            select! {
                x = self.rx_receiver.recv() => match x {
                    Some((domid, data)) => {
                        if let Some(data) = data {
                            let buffer = buffers.entry(domid).or_insert_with_key(|_| BytesMut::new());
                            buffer.extend_from_slice(&data);
                            if buffer.len() < 6 {
                                continue;
                            }

                            if buffer[0] != 0xff || buffer[1] != 0xff {
                                buffer.clear();
                                continue;
                            }

                            let size = (buffer[2] as u32 | (buffer[3] as u32) << 8 | (buffer[4] as u32) << 16 | (buffer[5] as u32) << 24) as usize;
                            let needed = size + 6;
                            if buffer.len() < needed {
                                continue;
                            }
                            let mut packet = buffer.split_to(needed);
                            packet.advance(6);
                            match IdmPacket::decode(packet) {
                                Ok(packet) => {
                                    let _ = client_or_create(domid, &self.tx_sender, &self.clients, &self.feeds).await?;
                                    let guard = self.feeds.lock().await;
                                    if let Some(feed) = guard.get(&domid) {
                                        let _ = feed.try_send(packet.clone());
                                    }
                                    let _ = self.snoop_sender.send(DaemonIdmSnoopPacket { from: domid, to: 0, packet });
                                }

                                Err(packet) => {
                                    warn!("received invalid packet from domain {}: {}", domid, packet);
                                }
                            }
                        } else {
                            let mut clients = self.clients.lock().await;
                            let mut feeds = self.feeds.lock().await;
                            clients.remove(&domid);
                            feeds.remove(&domid);
                        }
                    },

                    None => {
                        break;
                    }
                },
                x = self.tx_receiver.recv() => match x {
                    Some((domid, packet)) => {
                        let data = packet.encode_to_vec();
                        let mut buffer = vec![0u8; 6];
                        let length = data.len() as u32;
                        buffer[0] = 0xff;
                        buffer[1] = 0xff;
                        buffer[2] = length as u8;
                        buffer[3] = (length << 8) as u8;
                        buffer[4] = (length << 16) as u8;
                        buffer[5] = (length << 24) as u8;
                        buffer.extend_from_slice(&data);
                        self.tx_raw_sender.send((domid, buffer)).await?;
                        let _ = self.snoop_sender.send(DaemonIdmSnoopPacket { from: 0, to: domid, packet });
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

async fn client_or_create(
    domid: u32,
    tx_sender: &Sender<(u32, IdmPacket)>,
    clients: &ClientMap,
    feeds: &BackendFeedMap,
) -> Result<IdmClient> {
    let mut clients = clients.lock().await;
    let mut feeds = feeds.lock().await;
    match clients.entry(domid) {
        Entry::Occupied(entry) => Ok(entry.get().clone()),
        Entry::Vacant(entry) => {
            let (rx_sender, rx_receiver) = channel(100);
            feeds.insert(domid, rx_sender);
            let backend = IdmDaemonBackend {
                domid,
                rx_receiver,
                tx_sender: tx_sender.clone(),
            };
            let client = IdmClient::new(Box::new(backend) as Box<dyn IdmBackend>).await?;
            entry.insert(client.clone());
            Ok(client)
        }
    }
}

pub struct IdmDaemonBackend {
    domid: u32,
    rx_receiver: Receiver<IdmPacket>,
    tx_sender: Sender<(u32, IdmPacket)>,
}

#[async_trait::async_trait]
impl IdmBackend for IdmDaemonBackend {
    async fn recv(&mut self) -> Result<IdmPacket> {
        if let Some(packet) = self.rx_receiver.recv().await {
            Ok(packet)
        } else {
            Err(anyhow!("idm receive channel closed"))
        }
    }

    async fn send(&mut self, packet: IdmPacket) -> Result<()> {
        self.tx_sender.send((self.domid, packet)).await?;
        Ok(())
    }
}
