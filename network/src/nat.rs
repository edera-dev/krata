use anyhow::Result;
use async_trait::async_trait;
use etherparse::Ethernet2Slice;
use etherparse::IpNumber;
use etherparse::IpPayloadSlice;
use etherparse::Ipv4Slice;
use etherparse::Ipv6Slice;
use etherparse::LinkSlice;
use etherparse::NetSlice;
use etherparse::SlicedPacket;
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

#[async_trait]
pub trait NatHandler: Send {
    async fn receive(&self, packet: &[u8]) -> Result<()>;
}

#[async_trait]
pub trait NatHandlerFactory: Send {
    async fn nat(
        &self,
        key: NatKey,
        tx_sender: Sender<Vec<u8>>,
        reclaim_sender: Sender<NatKey>,
    ) -> Option<Box<dyn NatHandler>>;
}

pub struct NatTable {
    inner: HashMap<NatKey, Box<dyn NatHandler>>,
}

impl NatTable {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }
}

pub struct NatRouter {
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
        factory: Box<dyn NatHandlerFactory>,
        local_mac: EthernetAddress,
        local_cidrs: Vec<IpCidr>,
        tx_sender: Sender<Vec<u8>>,
    ) -> Self {
        let (reclaim_sender, reclaim_receiver) = channel(4);
        Self {
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
            self.table.inner.remove(&key);
            debug!("reclaimed nat key: {}", key);
            Some(key)
        } else {
            None
        })
    }

    pub async fn process(&mut self, data: &[u8]) -> Result<()> {
        let packet = SlicedPacket::from_ethernet(data)?;
        let Some(ref link) = packet.link else {
            return Ok(());
        };

        let LinkSlice::Ethernet2(ref ether) = link else {
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

        let Some(ref net) = packet.net else {
            return Ok(());
        };

        match net {
            NetSlice::Ipv4(ipv4) => self.process_ipv4(data, ether, ipv4).await?,
            NetSlice::Ipv6(ipv6) => self.process_ipv6(data, ether, ipv6).await?,
        }

        Ok(())
    }

    pub async fn process_ipv4<'a>(
        &mut self,
        data: &[u8],
        ether: &Ethernet2Slice<'a>,
        ipv4: &Ipv4Slice<'a>,
    ) -> Result<()> {
        let source_addr = IpAddress::Ipv4(ipv4.header().source_addr().into());
        let dest_addr = IpAddress::Ipv4(ipv4.header().destination_addr().into());
        match ipv4.header().protocol() {
            IpNumber::TCP => {
                self.process_tcp(data, ether, source_addr, dest_addr, ipv4.payload())
                    .await?;
            }

            IpNumber::UDP => {
                self.process_udp(data, ether, source_addr, dest_addr, ipv4.payload())
                    .await?;
            }

            _ => {}
        }

        Ok(())
    }

    pub async fn process_ipv6<'a>(
        &mut self,
        data: &[u8],
        ether: &Ethernet2Slice<'a>,
        ipv6: &Ipv6Slice<'a>,
    ) -> Result<()> {
        let source_addr = IpAddress::Ipv6(ipv6.header().source_addr().into());
        let dest_addr = IpAddress::Ipv6(ipv6.header().destination_addr().into());
        match ipv6.header().next_header() {
            IpNumber::TCP => {
                self.process_tcp(data, ether, source_addr, dest_addr, ipv6.payload())
                    .await?;
            }

            IpNumber::UDP => {
                self.process_udp(data, ether, source_addr, dest_addr, ipv6.payload())
                    .await?;
            }

            _ => {}
        }

        Ok(())
    }

    pub async fn process_tcp<'a>(
        &mut self,
        data: &'a [u8],
        ether: &Ethernet2Slice<'a>,
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<()> {
        let header = TcpHeaderSlice::from_slice(payload.payload)?;
        let source = IpEndpoint::new(source_addr, header.source_port());
        let dest = IpEndpoint::new(dest_addr, header.destination_port());
        let key = NatKey {
            protocol: NatKeyProtocol::Tcp,
            client_mac: EthernetAddress(ether.source()),
            local_mac: EthernetAddress(ether.destination()),
            client_ip: source,
            external_ip: dest,
        };
        self.process_nat(data, key).await?;
        Ok(())
    }

    pub async fn process_udp<'a>(
        &mut self,
        data: &'a [u8],
        ether: &Ethernet2Slice<'a>,
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<()> {
        let header = UdpHeaderSlice::from_slice(payload.payload)?;
        let source = IpEndpoint::new(source_addr, header.source_port());
        let dest = IpEndpoint::new(dest_addr, header.destination_port());
        let key = NatKey {
            protocol: NatKeyProtocol::Udp,
            client_mac: EthernetAddress(ether.source()),
            local_mac: EthernetAddress(ether.destination()),
            client_ip: source,
            external_ip: dest,
        };
        self.process_nat(data, key).await?;
        Ok(())
    }

    pub async fn process_nat(&mut self, data: &[u8], key: NatKey) -> Result<()> {
        for cidr in &self.local_cidrs {
            if cidr.contains_addr(&key.external_ip.addr) {
                return Ok(());
            }
        }

        let handler: Option<&mut Box<dyn NatHandler>> = match self.table.inner.entry(key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                if let Some(handler) = self
                    .factory
                    .nat(key, self.tx_sender.clone(), self.reclaim_sender.clone())
                    .await
                {
                    debug!("creating nat entry for key: {}", key);
                    Some(entry.insert(handler))
                } else {
                    None
                }
            }
        };

        if let Some(handler) = handler {
            handler.receive(data).await?;
        }
        Ok(())
    }
}
