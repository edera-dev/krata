use crate::pkt::RecvPacket;
use crate::pkt::RecvPacketIp;
use anyhow::Result;
use bytes::BytesMut;
use etherparse::Icmpv4Header;
use etherparse::Icmpv4Type;
use etherparse::Icmpv6Header;
use etherparse::Icmpv6Type;
use etherparse::IpNumber;
use etherparse::IpPayloadSlice;
use etherparse::Ipv4Slice;
use etherparse::Ipv6Slice;
use etherparse::SlicedPacket;
use etherparse::TcpHeaderSlice;
use etherparse::UdpHeaderSlice;
use log::warn;
use log::{debug, trace};
use smoltcp::wire::EthernetAddress;
use smoltcp::wire::IpAddress;
use smoltcp::wire::IpCidr;
use smoltcp::wire::IpEndpoint;
use std::collections::hash_map::Entry;
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;
use tokio::task::JoinHandle;

use super::handler::NatHandler;
use super::handler::NatHandlerContext;
use super::handler::NatHandlerFactory;
use super::key::NatKey;
use super::key::NatKeyProtocol;
use super::table::NatTable;

const RECEIVE_CHANNEL_QUEUE_LEN: usize = 3000;
const RECLAIM_CHANNEL_QUEUE_LEN: usize = 30;

pub struct NatProcessor {
    mtu: usize,
    local_mac: EthernetAddress,
    local_cidrs: Vec<IpCidr>,
    table: NatTable,
    factory: Box<dyn NatHandlerFactory>,
    transmit_sender: Sender<BytesMut>,
    reclaim_sender: Sender<NatKey>,
    reclaim_receiver: Receiver<NatKey>,
    receive_receiver: Receiver<BytesMut>,
}

enum NatProcessorSelect {
    Reclaim(Option<NatKey>),
    ReceivedPacket(Option<BytesMut>),
}

impl NatProcessor {
    pub fn launch(
        mtu: usize,
        factory: Box<dyn NatHandlerFactory>,
        local_mac: EthernetAddress,
        local_cidrs: Vec<IpCidr>,
        transmit_sender: Sender<BytesMut>,
    ) -> Result<(Sender<BytesMut>, JoinHandle<()>)> {
        let (reclaim_sender, reclaim_receiver) = channel(RECLAIM_CHANNEL_QUEUE_LEN);
        let (receive_sender, receive_receiver) = channel(RECEIVE_CHANNEL_QUEUE_LEN);
        let mut processor = Self {
            mtu,
            local_mac,
            local_cidrs,
            factory,
            table: NatTable::new(),
            transmit_sender,
            reclaim_sender,
            receive_receiver,
            reclaim_receiver,
        };

        let handle = tokio::task::spawn(async move {
            if let Err(error) = processor.process().await {
                warn!("nat processing failed: {}", error);
            }
        });

        Ok((receive_sender, handle))
    }

    pub async fn process(&mut self) -> Result<()> {
        loop {
            let selection = select! {
                x = self.reclaim_receiver.recv() => NatProcessorSelect::Reclaim(x),
                x = self.receive_receiver.recv() => NatProcessorSelect::ReceivedPacket(x),
            };

            match selection {
                NatProcessorSelect::Reclaim(Some(key)) => {
                    if self.table.inner.remove(&key).is_some() {
                        debug!("reclaimed nat key: {}", key);
                    }
                }

                NatProcessorSelect::ReceivedPacket(Some(packet)) => {
                    if let Ok(slice) = SlicedPacket::from_ethernet(&packet) {
                        let Ok(packet) = RecvPacket::new(&packet, &slice) else {
                            continue;
                        };

                        self.process_packet(&packet).await?;
                    }
                }

                NatProcessorSelect::ReceivedPacket(None) | NatProcessorSelect::Reclaim(None) => {
                    break
                }
            }
        }
        Ok(())
    }

    pub async fn process_reclaim(&mut self) -> Result<Option<NatKey>> {
        Ok(if let Some(key) = self.reclaim_receiver.recv().await {
            if self.table.inner.remove(&key).is_some() {
                debug!("reclaimed nat key: {}", key);
                Some(key)
            } else {
                None
            }
        } else {
            None
        })
    }

    pub async fn process_packet<'a>(&mut self, packet: &RecvPacket<'a>) -> Result<()> {
        let Some(ether) = packet.ether else {
            return Ok(());
        };

        let mac = EthernetAddress(ether.destination());
        if mac != self.local_mac {
            trace!(
                "received packet with destination {} which is not the local mac {}",
                mac,
                self.local_mac
            );
            return Ok(());
        }

        let key = match packet.ip {
            Some(RecvPacketIp::Ipv4(ipv4)) => self.extract_key_ipv4(packet, ipv4)?,
            Some(RecvPacketIp::Ipv6(ipv6)) => self.extract_key_ipv6(packet, ipv6)?,
            _ => None,
        };

        let Some(key) = key else {
            return Ok(());
        };

        for cidr in &self.local_cidrs {
            if cidr.contains_addr(&key.external_ip.addr) {
                return Ok(());
            }
        }

        let context = NatHandlerContext {
            mtu: self.mtu,
            key,
            transmit_sender: self.transmit_sender.clone(),
            reclaim_sender: self.reclaim_sender.clone(),
        };
        let handler: Option<&mut Box<dyn NatHandler>> = match self.table.inner.entry(key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                if let Some(handler) = self.factory.nat(context).await {
                    debug!("creating nat entry for key: {}", key);
                    Some(entry.insert(handler))
                } else {
                    None
                }
            }
        };

        if let Some(handler) = handler {
            if !handler.receive(packet.raw).await? {
                self.reclaim_sender.try_send(key)?;
            }
        }
        Ok(())
    }

    pub fn extract_key_ipv4<'a>(
        &mut self,
        packet: &RecvPacket<'a>,
        ipv4: &Ipv4Slice<'a>,
    ) -> Result<Option<NatKey>> {
        let source_addr = IpAddress::Ipv4(ipv4.header().source_addr().into());
        let dest_addr = IpAddress::Ipv4(ipv4.header().destination_addr().into());
        Ok(match ipv4.header().protocol() {
            IpNumber::TCP => {
                self.extract_key_tcp(packet, source_addr, dest_addr, ipv4.payload())?
            }

            IpNumber::UDP => {
                self.extract_key_udp(packet, source_addr, dest_addr, ipv4.payload())?
            }

            IpNumber::ICMP => {
                self.extract_key_icmpv4(packet, source_addr, dest_addr, ipv4.payload())?
            }

            _ => None,
        })
    }

    pub fn extract_key_ipv6<'a>(
        &mut self,
        packet: &RecvPacket<'a>,
        ipv6: &Ipv6Slice<'a>,
    ) -> Result<Option<NatKey>> {
        let source_addr = IpAddress::Ipv6(ipv6.header().source_addr().into());
        let dest_addr = IpAddress::Ipv6(ipv6.header().destination_addr().into());
        Ok(match ipv6.header().next_header() {
            IpNumber::TCP => {
                self.extract_key_tcp(packet, source_addr, dest_addr, ipv6.payload())?
            }

            IpNumber::UDP => {
                self.extract_key_udp(packet, source_addr, dest_addr, ipv6.payload())?
            }

            IpNumber::IPV6_ICMP => {
                self.extract_key_icmpv6(packet, source_addr, dest_addr, ipv6.payload())?
            }

            _ => None,
        })
    }

    pub fn extract_key_udp<'a>(
        &mut self,
        packet: &RecvPacket<'a>,
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<Option<NatKey>> {
        let Some(ether) = packet.ether else {
            return Ok(None);
        };
        let header = UdpHeaderSlice::from_slice(payload.payload)?;
        let source = IpEndpoint::new(source_addr, header.source_port());
        let dest = IpEndpoint::new(dest_addr, header.destination_port());
        Ok(Some(NatKey {
            protocol: NatKeyProtocol::Udp,
            client_mac: EthernetAddress(ether.source()),
            local_mac: EthernetAddress(ether.destination()),
            client_ip: source,
            external_ip: dest,
        }))
    }

    pub fn extract_key_icmpv4<'a>(
        &mut self,
        packet: &RecvPacket<'a>,
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<Option<NatKey>> {
        let Some(ether) = packet.ether else {
            return Ok(None);
        };
        let (header, _) = Icmpv4Header::from_slice(payload.payload)?;
        let Icmpv4Type::EchoRequest(_) = header.icmp_type else {
            return Ok(None);
        };
        let source = IpEndpoint::new(source_addr, 0);
        let dest = IpEndpoint::new(dest_addr, 0);
        Ok(Some(NatKey {
            protocol: NatKeyProtocol::Icmp,
            client_mac: EthernetAddress(ether.source()),
            local_mac: EthernetAddress(ether.destination()),
            client_ip: source,
            external_ip: dest,
        }))
    }

    pub fn extract_key_icmpv6<'a>(
        &mut self,
        packet: &RecvPacket<'a>,
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<Option<NatKey>> {
        let Some(ether) = packet.ether else {
            return Ok(None);
        };
        let (header, _) = Icmpv6Header::from_slice(payload.payload)?;
        let Icmpv6Type::EchoRequest(_) = header.icmp_type else {
            return Ok(None);
        };
        let source = IpEndpoint::new(source_addr, 0);
        let dest = IpEndpoint::new(dest_addr, 0);
        Ok(Some(NatKey {
            protocol: NatKeyProtocol::Icmp,
            client_mac: EthernetAddress(ether.source()),
            local_mac: EthernetAddress(ether.destination()),
            client_ip: source,
            external_ip: dest,
        }))
    }

    pub fn extract_key_tcp<'a>(
        &mut self,
        packet: &RecvPacket<'a>,
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<Option<NatKey>> {
        let Some(ether) = packet.ether else {
            return Ok(None);
        };
        let header = TcpHeaderSlice::from_slice(payload.payload)?;
        let source = IpEndpoint::new(source_addr, header.source_port());
        let dest = IpEndpoint::new(dest_addr, header.destination_port());
        Ok(Some(NatKey {
            protocol: NatKeyProtocol::Tcp,
            client_mac: EthernetAddress(ether.source()),
            local_mac: EthernetAddress(ether.destination()),
            client_ip: source,
            external_ip: dest,
        }))
    }
}
