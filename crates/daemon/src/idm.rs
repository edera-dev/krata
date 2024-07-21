use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};

use anyhow::{anyhow, Result};
use bytes::{Buf, BytesMut};
use krata::idm::{
    client::{IdmBackend, IdmInternalClient},
    internal::INTERNAL_IDM_CHANNEL,
    transport::IdmTransportPacket,
};
use kratart::channel::ChannelService;
use log::{debug, error, warn};
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
use uuid::Uuid;

use crate::zlt::ZoneLookupTable;

type BackendFeedMap = Arc<Mutex<HashMap<u32, Sender<IdmTransportPacket>>>>;
type ClientMap = Arc<Mutex<HashMap<u32, IdmInternalClient>>>;

#[derive(Clone)]
pub struct DaemonIdmHandle {
    glt: ZoneLookupTable,
    clients: ClientMap,
    feeds: BackendFeedMap,
    tx_sender: Sender<(u32, IdmTransportPacket)>,
    task: Arc<JoinHandle<()>>,
    snoop_sender: broadcast::Sender<DaemonIdmSnoopPacket>,
}

impl DaemonIdmHandle {
    pub fn snoop(&self) -> broadcast::Receiver<DaemonIdmSnoopPacket> {
        self.snoop_sender.subscribe()
    }

    pub async fn client(&self, uuid: Uuid) -> Result<IdmInternalClient> {
        let Some(domid) = self.glt.lookup_domid_by_uuid(&uuid).await else {
            return Err(anyhow!("unable to find domain {}", uuid));
        };
        self.client_by_domid(domid).await
    }

    pub async fn client_by_domid(&self, domid: u32) -> Result<IdmInternalClient> {
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
    pub packet: IdmTransportPacket,
}

pub struct DaemonIdm {
    glt: ZoneLookupTable,
    clients: ClientMap,
    feeds: BackendFeedMap,
    tx_sender: Sender<(u32, IdmTransportPacket)>,
    tx_raw_sender: Sender<(u32, Vec<u8>)>,
    tx_receiver: Receiver<(u32, IdmTransportPacket)>,
    rx_receiver: Receiver<(u32, Option<Vec<u8>>)>,
    snoop_sender: broadcast::Sender<DaemonIdmSnoopPacket>,
    task: JoinHandle<()>,
}

impl DaemonIdm {
    pub async fn new(glt: ZoneLookupTable) -> Result<DaemonIdm> {
        debug!("allocating channel for IDM");
        let (service, tx_raw_sender, rx_receiver) =
            ChannelService::new("krata-channel".to_string(), None).await?;
        let (tx_sender, tx_receiver) = channel(100);
        let (snoop_sender, _) = broadcast::channel(100);

        debug!("starting channel service");
        let task = service.launch().await?;

        let clients = Arc::new(Mutex::new(HashMap::new()));
        let feeds = Arc::new(Mutex::new(HashMap::new()));

        Ok(DaemonIdm {
            glt,
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
        let glt = self.glt.clone();
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
            glt,
            clients,
            feeds,
            tx_sender,
            snoop_sender,
            task: Arc::new(task),
        })
    }

    async fn process_rx_packet(
        &mut self,
        domid: u32,
        data: Option<Vec<u8>>,
        buffers: &mut HashMap<u32, BytesMut>,
    ) -> Result<()> {
        if let Some(data) = data {
            let buffer = buffers.entry(domid).or_insert_with_key(|_| BytesMut::new());
            buffer.extend_from_slice(&data);
            loop {
                if buffer.len() < 6 {
                    break;
                }

                if buffer[0] != 0xff || buffer[1] != 0xff {
                    buffer.clear();
                    break;
                }

                let size = (buffer[2] as u32 | (buffer[3] as u32) << 8 | (buffer[4] as u32) << 16 | (buffer[5] as u32) << 24) as usize;
                let needed = size + 6;
                if buffer.len() < needed {
                    break;
                }
                let mut packet = buffer.split_to(needed);
                packet.advance(6);
                match IdmTransportPacket::decode(packet) {
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
            }
        } else {
            let mut clients = self.clients.lock().await;
            let mut feeds = self.feeds.lock().await;
            clients.remove(&domid);
            feeds.remove(&domid);
        }
        Ok(())
    }

    async fn tx_packet(&mut self, domid: u32, packet: IdmTransportPacket) -> Result<()> {
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
        let _ = self.snoop_sender.send(DaemonIdmSnoopPacket {
            from: 0,
            to: domid,
            packet,
        });
        Ok(())
    }

    async fn process(&mut self, buffers: &mut HashMap<u32, BytesMut>) -> Result<()> {
        loop {
            select! {
                x = self.rx_receiver.recv() => match x {
                    Some((domid, data)) => {
                        self.process_rx_packet(domid, data, buffers).await?;
                    },

                    None => {
                        break;
                    }
                },
                x = self.tx_receiver.recv() => match x {
                    Some((domid, packet)) => {
                        self.tx_packet(domid, packet).await?;
                    },

                    None => {
                        break;
                    }
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

async fn client_or_create(
    domid: u32,
    tx_sender: &Sender<(u32, IdmTransportPacket)>,
    clients: &ClientMap,
    feeds: &BackendFeedMap,
) -> Result<IdmInternalClient> {
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
            let client = IdmInternalClient::new(
                INTERNAL_IDM_CHANNEL,
                Box::new(backend) as Box<dyn IdmBackend>,
            )
            .await?;
            entry.insert(client.clone());
            Ok(client)
        }
    }
}

pub struct IdmDaemonBackend {
    domid: u32,
    rx_receiver: Receiver<IdmTransportPacket>,
    tx_sender: Sender<(u32, IdmTransportPacket)>,
}

#[async_trait::async_trait]
impl IdmBackend for IdmDaemonBackend {
    async fn recv(&mut self) -> Result<Vec<IdmTransportPacket>> {
        if let Some(packet) = self.rx_receiver.recv().await {
            Ok(vec![packet])
        } else {
            Err(anyhow!("idm receive channel closed"))
        }
    }

    async fn send(&mut self, packet: IdmTransportPacket) -> Result<()> {
        self.tx_sender.send((self.domid, packet)).await?;
        Ok(())
    }
}
