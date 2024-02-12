use crate::pkt::RecvPacket;
use crate::pkt::RecvPacketIp;
use anyhow::Result;
use async_trait::async_trait;
use etherparse::Icmpv4Header;
use etherparse::Icmpv4Type;
use etherparse::Icmpv6Header;
use etherparse::Icmpv6Type;
use etherparse::IpNumber;
use etherparse::IpPayloadSlice;
use etherparse::Ipv4Slice;
use etherparse::Ipv6Slice;
use etherparse::TcpHeaderSlice;
use etherparse::UdpHeaderSlice;
use log::{debug, trace};
use smoltcp::wire::EthernetAddress;
use smoltcp::wire::IpAddress;
use smoltcp::wire::IpCidr;
use smoltcp::wire::IpEndpoint;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::Display;
use tokio::sync::mpsc::channel;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub enum NatKeyProtocol {
    Tcp,
    Udp,
    Icmp,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct NatKey {
    pub protocol: NatKeyProtocol,
    pub client_mac: EthernetAddress,
    pub local_mac: EthernetAddress,
    pub client_ip: IpEndpoint,
    pub external_ip: IpEndpoint,
}

impl Display for NatKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} -> {} {:?} {} -> {}",
            self.client_mac, self.local_mac, self.protocol, self.client_ip, self.external_ip
        )
    }
}

#[derive(Debug)]
pub struct NatHandlerContext {
    pub mtu: usize,
    pub key: NatKey,
    tx_sender: Sender<Vec<u8>>,
    reclaim_sender: Sender<NatKey>,
}

impl NatHandlerContext {
    pub fn try_send(&self, buffer: Vec<u8>) -> Result<()> {
        self.tx_sender.try_send(buffer)?;
        Ok(())
    }

    pub async fn reclaim(&self) -> Result<()> {
        self.reclaim_sender.try_send(self.key)?;
        Ok(())
    }
}

#[async_trait]
pub trait NatHandler: Send {
    async fn receive(&self, packet: &[u8]) -> Result<bool>;
}

#[async_trait]
pub trait NatHandlerFactory: Send {
    async fn nat(&self, context: NatHandlerContext) -> Option<Box<dyn NatHandler>>;
}

pub struct NatTable {
    inner: HashMap<NatKey, Box<dyn NatHandler>>,
}

impl Default for NatTable {
    fn default() -> Self {
        Self::new()
    }
}

impl NatTable {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }
}

pub struct NatRouter {
    mtu: usize,
    local_mac: EthernetAddress,
    local_cidrs: Vec<IpCidr>,
    factory: Box<dyn NatHandlerFactory>,
    table: NatTable,
    tx_sender: Sender<Vec<u8>>,
    reclaim_sender: Sender<NatKey>,
    reclaim_receiver: Receiver<NatKey>,
}

impl NatRouter {
    pub fn new(
        mtu: usize,
        factory: Box<dyn NatHandlerFactory>,
        local_mac: EthernetAddress,
        local_cidrs: Vec<IpCidr>,
        tx_sender: Sender<Vec<u8>>,
    ) -> Self {
        let (reclaim_sender, reclaim_receiver) = channel(4);
        Self {
            mtu,
            local_mac,
            local_cidrs,
            factory,
            table: NatTable::new(),
            tx_sender,
            reclaim_sender,
            reclaim_receiver,
        }
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

    pub async fn process<'a>(&mut self, packet: &RecvPacket<'a>) -> Result<()> {
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
            tx_sender: self.tx_sender.clone(),
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
}
