use anyhow::{anyhow, Result};
use bytes::BytesMut;
use etherparse::{EtherType, Ethernet2Header, IpNumber, Ipv4Header, Ipv6Header, TcpHeader};
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

const TO_BRIDGE_QUEUE_LEN: usize = 3000;
const FROM_BRIDGE_QUEUE_LEN: usize = 3000;
const BROADCAST_QUEUE_LEN: usize = 3000;
const MEMBER_LEAVE_QUEUE_LEN: usize = 30;

#[derive(Debug)]
struct BridgeMember {
    pub from_bridge_sender: Sender<BytesMut>,
}

pub struct BridgeJoinHandle {
    mac: EthernetAddress,
    pub to_bridge_sender: Sender<BytesMut>,
    pub from_bridge_receiver: Receiver<BytesMut>,
    pub from_broadcast_receiver: BroadcastReceiver<BytesMut>,
    member_leave_sender: Sender<EthernetAddress>,
}

impl Drop for BridgeJoinHandle {
    fn drop(&mut self) {
        if let Err(error) = self.member_leave_sender.try_send(self.mac) {
            warn!(
                "virtual bridge member {} failed to leave: {}",
                self.mac, error
            );
        }
    }
}

type VirtualBridgeMemberMap = Arc<Mutex<HashMap<EthernetAddress, BridgeMember>>>;

#[derive(Clone)]
pub struct VirtualBridge {
    to_bridge_sender: Sender<BytesMut>,
    from_broadcast_sender: BroadcastSender<BytesMut>,
    member_leave_sender: Sender<EthernetAddress>,
    members: VirtualBridgeMemberMap,
    _task: Arc<JoinHandle<()>>,
}

enum VirtualBridgeSelect {
    BroadcastSent,
    PacketReceived(Option<BytesMut>),
    MemberLeave(Option<EthernetAddress>),
}

impl VirtualBridge {
    pub fn new() -> Result<VirtualBridge> {
        let (to_bridge_sender, to_bridge_receiver) = channel::<BytesMut>(TO_BRIDGE_QUEUE_LEN);
        let (member_leave_sender, member_leave_reciever) =
            channel::<EthernetAddress>(MEMBER_LEAVE_QUEUE_LEN);
        let (from_broadcast_sender, from_broadcast_receiver) =
            broadcast_channel(BROADCAST_QUEUE_LEN);

        let members = Arc::new(Mutex::new(HashMap::new()));
        let handle = {
            let members = members.clone();
            let broadcast_rx_sender = from_broadcast_sender.clone();
            tokio::task::spawn(async move {
                if let Err(error) = VirtualBridge::process(
                    members,
                    member_leave_reciever,
                    to_bridge_receiver,
                    broadcast_rx_sender,
                    from_broadcast_receiver,
                )
                .await
                {
                    warn!("virtual bridge processing task failed: {}", error);
                }
            })
        };

        Ok(VirtualBridge {
            to_bridge_sender,
            from_broadcast_sender,
            member_leave_sender,
            members,
            _task: Arc::new(handle),
        })
    }

    pub async fn join(&self, mac: EthernetAddress) -> Result<BridgeJoinHandle> {
        let (from_bridge_sender, from_bridge_receiver) = channel::<BytesMut>(FROM_BRIDGE_QUEUE_LEN);
        let member = BridgeMember { from_bridge_sender };

        match self.members.lock().await.entry(mac) {
            Entry::Occupied(_) => {
                return Err(anyhow!("virtual bridge member {} already exists", mac));
            }
            Entry::Vacant(entry) => {
                entry.insert(member);
            }
        };
        debug!("virtual bridge member {} has joined", mac);
        Ok(BridgeJoinHandle {
            mac,
            member_leave_sender: self.member_leave_sender.clone(),
            from_bridge_receiver,
            from_broadcast_receiver: self.from_broadcast_sender.subscribe(),
            to_bridge_sender: self.to_bridge_sender.clone(),
        })
    }

    async fn process(
        members: VirtualBridgeMemberMap,
        mut member_leave_reciever: Receiver<EthernetAddress>,
        mut to_bridge_receiver: Receiver<BytesMut>,
        broadcast_rx_sender: BroadcastSender<BytesMut>,
        mut from_broadcast_receiver: BroadcastReceiver<BytesMut>,
    ) -> Result<()> {
        loop {
            let selection = select! {
                biased;
                _ = from_broadcast_receiver.recv() => VirtualBridgeSelect::BroadcastSent,
                x = to_bridge_receiver.recv() => VirtualBridgeSelect::PacketReceived(x),
                x = member_leave_reciever.recv() => VirtualBridgeSelect::MemberLeave(x),
            };

            match selection {
                VirtualBridgeSelect::PacketReceived(Some(mut packet)) => {
                    let (header, payload) = match Ethernet2Header::from_slice(&packet) {
                        Ok(data) => data,
                        Err(error) => {
                            debug!("virtual bridge failed to parse ethernet header: {}", error);
                            continue;
                        }
                    };

                    // recalculate TCP checksums when routing packets.
                    // the xen network backend / frontend drivers for linux
                    // use checksum offloading but since we bypass some layers
                    // of the kernel we have to do it ourselves.
                    if header.ether_type == EtherType::IPV4 {
                        let (ipv4, payload) = Ipv4Header::from_slice(payload)?;
                        if ipv4.protocol == IpNumber::TCP {
                            let (mut tcp, payload) = TcpHeader::from_slice(payload)?;
                            tcp.checksum = tcp.calc_checksum_ipv4(&ipv4, payload)?;
                            let tcp_header_offset = Ethernet2Header::LEN + ipv4.header_len();
                            let tcp_header_bytes = tcp.to_bytes();
                            for (i, b) in tcp_header_bytes.iter().enumerate() {
                                packet[tcp_header_offset + i] = *b;
                            }
                        }
                    } else if header.ether_type == EtherType::IPV6 {
                        let (ipv6, payload) = Ipv6Header::from_slice(payload)?;
                        if ipv6.next_header == IpNumber::TCP {
                            let (mut tcp, payload) = TcpHeader::from_slice(payload)?;
                            tcp.checksum = tcp.calc_checksum_ipv6(&ipv6, payload)?;
                            let tcp_header_offset = Ethernet2Header::LEN + ipv6.header_len();
                            let tcp_header_bytes = tcp.to_bytes();
                            for (i, b) in tcp_header_bytes.iter().enumerate() {
                                packet[tcp_header_offset + i] = *b;
                            }
                        }
                    }

                    let destination = EthernetAddress(header.destination);
                    if destination.is_multicast() {
                        broadcast_rx_sender.send(packet)?;
                        continue;
                    }
                    match members.lock().await.get(&destination) {
                        Some(member) => {
                            member.from_bridge_sender.try_send(packet)?;
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

                VirtualBridgeSelect::MemberLeave(Some(mac)) => {
                    if members.lock().await.remove(&mac).is_some() {
                        debug!("virtual bridge member {} has left", mac);
                    }
                }

                VirtualBridgeSelect::PacketReceived(None) => break,
                VirtualBridgeSelect::MemberLeave(None) => {}
                VirtualBridgeSelect::BroadcastSent => {}
            }
        }
        Ok(())
    }
}
