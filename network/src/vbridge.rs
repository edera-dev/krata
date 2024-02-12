use anyhow::{anyhow, Result};
use bytes::BytesMut;
use etherparse::Ethernet2Header;
use log::{debug, trace, warn};
use smoltcp::wire::EthernetAddress;
use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
};
use tokio::sync::broadcast::{
    channel as broadcast_channel, Receiver as BroadcastReceiver, Sender as BroadcastSender,
};
use tokio::{
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

const BROADCAST_MAC_ADDR: &[u8; 6] = &[0xff; 6];

const BRIDGE_TX_QUEUE_LEN: usize = 4;
const BRIDGE_RX_QUEUE_LEN: usize = 4;
const BROADCAST_RX_QUEUE_LEN: usize = 4;

#[derive(Debug)]
struct BridgeMember {
    pub bridge_rx_sender: Sender<BytesMut>,
}

pub struct BridgeJoinHandle {
    pub bridge_tx_sender: Sender<BytesMut>,
    pub bridge_rx_receiver: Receiver<BytesMut>,
    pub broadcast_rx_receiver: BroadcastReceiver<BytesMut>,
}

type VirtualBridgeMemberMap = Arc<Mutex<HashMap<[u8; 6], BridgeMember>>>;

#[derive(Clone)]
pub struct VirtualBridge {
    members: VirtualBridgeMemberMap,
    bridge_tx_sender: Sender<BytesMut>,
    broadcast_rx_sender: BroadcastSender<BytesMut>,
    _task: Arc<JoinHandle<()>>,
}

enum VirtualBridgeSelect {
    BroadcastSent(Option<BytesMut>),
    PacketReceived(Option<BytesMut>),
}

impl VirtualBridge {
    pub fn new() -> Result<VirtualBridge> {
        let (bridge_tx_sender, bridge_tx_receiver) = channel::<BytesMut>(BRIDGE_TX_QUEUE_LEN);
        let (broadcast_rx_sender, broadcast_rx_receiver) =
            broadcast_channel(BROADCAST_RX_QUEUE_LEN);

        let members = Arc::new(Mutex::new(HashMap::new()));
        let handle = {
            let members = members.clone();
            let broadcast_rx_sender = broadcast_rx_sender.clone();
            tokio::task::spawn(async move {
                if let Err(error) = VirtualBridge::process(
                    members,
                    bridge_tx_receiver,
                    broadcast_rx_sender,
                    broadcast_rx_receiver,
                )
                .await
                {
                    warn!("virtual bridge processing task failed: {}", error);
                }
            })
        };

        Ok(VirtualBridge {
            bridge_tx_sender,
            members,
            broadcast_rx_sender,
            _task: Arc::new(handle),
        })
    }

    pub async fn join(&self, mac: EthernetAddress) -> Result<BridgeJoinHandle> {
        let (bridge_rx_sender, bridge_rx_receiver) = channel::<BytesMut>(BRIDGE_RX_QUEUE_LEN);
        let member = BridgeMember { bridge_rx_sender };

        match self.members.lock().await.entry(mac.0) {
            Entry::Occupied(_) => {
                return Err(anyhow!(
                    "virtual bridge already has a member with address {}",
                    mac
                ));
            }
            Entry::Vacant(entry) => {
                entry.insert(member);
            }
        };
        debug!("virtual bridge member has joined: {}", mac);
        Ok(BridgeJoinHandle {
            bridge_rx_receiver,
            broadcast_rx_receiver: self.broadcast_rx_sender.subscribe(),
            bridge_tx_sender: self.bridge_tx_sender.clone(),
        })
    }

    async fn process(
        members: VirtualBridgeMemberMap,
        mut bridge_tx_receiver: Receiver<BytesMut>,
        broadcast_rx_sender: BroadcastSender<BytesMut>,
        mut broadcast_rx_receiver: BroadcastReceiver<BytesMut>,
    ) -> Result<()> {
        loop {
            let selection = select! {
                biased;
                x = bridge_tx_receiver.recv() => VirtualBridgeSelect::PacketReceived(x),
                x = broadcast_rx_receiver.recv() => VirtualBridgeSelect::BroadcastSent(x.ok()),
            };

            match selection {
                VirtualBridgeSelect::PacketReceived(Some(packet)) => {
                    let header = match Ethernet2Header::from_slice(&packet) {
                        Ok((header, _)) => header,
                        Err(error) => {
                            debug!("virtual bridge failed to parse ethernet header: {}", error);
                            continue;
                        }
                    };

                    let destination = &header.destination;
                    if destination == BROADCAST_MAC_ADDR {
                        trace!(
                            "broadcasting bridged packet from {}",
                            EthernetAddress(header.source)
                        );
                        broadcast_rx_sender.send(packet)?;
                        continue;
                    }
                    match members.lock().await.get(destination) {
                        Some(member) => {
                            member.bridge_rx_sender.try_send(packet)?;
                            trace!(
                                "sending bridged packet from {} to {}",
                                EthernetAddress(header.source),
                                EthernetAddress(header.destination)
                            );
                        }
                        None => {
                            trace!(
                                "no bridge member with address: {}",
                                EthernetAddress(*destination)
                            );
                        }
                    }
                }

                VirtualBridgeSelect::PacketReceived(None) => break,
                VirtualBridgeSelect::BroadcastSent(_) => {}
            }
        }
        Ok(())
    }
}
