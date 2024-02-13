use anyhow::{anyhow, Result};
use bytes::BytesMut;
use etherparse::{EtherType, Ethernet2Header, IpNumber, Ipv4Header, TcpHeader};
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

const BRIDGE_TX_QUEUE_LEN: usize = 50;
const BRIDGE_RX_QUEUE_LEN: usize = 50;
const BROADCAST_RX_QUEUE_LEN: usize = 50;

#[derive(Debug)]
struct BridgeMember {
    pub bridge_rx_sender: Sender<BytesMut>,
}

pub struct BridgeJoinHandle {
    pub bridge_tx_sender: Sender<BytesMut>,
    pub bridge_rx_receiver: Receiver<BytesMut>,
    pub broadcast_rx_receiver: BroadcastReceiver<BytesMut>,
}

type VirtualBridgeMemberMap = Arc<Mutex<HashMap<EthernetAddress, BridgeMember>>>;

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

        match self.members.lock().await.entry(mac) {
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
                    let mut packet: Vec<u8> = packet.into();
                    let (header, payload) = match Ethernet2Header::from_slice(&packet) {
                        Ok(data) => data,
                        Err(error) => {
                            debug!("virtual bridge failed to parse ethernet header: {}", error);
                            continue;
                        }
                    };

                    if header.ether_type == EtherType::IPV4 {
                        let (ipv4, payload) = Ipv4Header::from_slice(payload)?;

                        // recalculate TCP checksums when routing packets.
                        // the xen network backend / frontend drivers for linux
                        // are very stupid and do not calculate these properly
                        // despite all best attempts at making it do so.
                        if ipv4.protocol == IpNumber::TCP {
                            let (mut tcp, payload) = TcpHeader::from_slice(payload)?;
                            tcp.checksum = tcp.calc_checksum_ipv4(&ipv4, payload)?;
                            let tcp_header_offset = Ethernet2Header::LEN + ipv4.header_len();
                            let tcp_header_bytes = tcp.to_bytes();
                            for (i, b) in tcp_header_bytes.iter().enumerate() {
                                packet[tcp_header_offset + i] = *b;
                            }
                        }
                    }

                    let destination = EthernetAddress(header.destination);
                    if destination.is_multicast() {
                        trace!(
                            "broadcasting bridged packet from {}",
                            EthernetAddress(header.source)
                        );
                        broadcast_rx_sender.send(packet.as_slice().into())?;
                        continue;
                    }
                    match members.lock().await.get(&destination) {
                        Some(member) => {
                            member.bridge_rx_sender.try_send(packet.as_slice().into())?;
                            trace!(
                                "sending bridged packet from {} to {}",
                                EthernetAddress(header.source),
                                EthernetAddress(header.destination)
                            );
                        }
                        None => {
                            trace!("no bridge member with address: {}", destination);
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
