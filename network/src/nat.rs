// Referenced https://github.com/vi/wgslirpy/blob/master/crates/libwgslirpy/src/router.rs as a very interesting way to implement NAT.
// hypha will heavily change how the original code functions however. NatKey was a very useful example of what we need to store in a NAT map.

use anyhow::Result;
use async_trait::async_trait;
use etherparse::IpNumber;
use etherparse::IpPayloadSlice;
use etherparse::Ipv4Slice;
use etherparse::LinkSlice;
use etherparse::NetSlice;
use etherparse::SlicedPacket;
use etherparse::TcpHeaderSlice;
use etherparse::UdpHeaderSlice;
use smoltcp::wire::EthernetAddress;
use smoltcp::wire::IpAddress;
use smoltcp::wire::IpEndpoint;
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::fmt::Display;

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub enum NatKey {
    Tcp {
        client: IpEndpoint,
        external: IpEndpoint,
    },

    Udp {
        client: IpEndpoint,
        external: IpEndpoint,
    },

    Ping {
        client: IpAddress,
        external: IpAddress,
    },
}

impl Display for NatKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            NatKey::Tcp { client, external } => write!(f, "TCP {client} -> {external}"),
            NatKey::Udp { client, external } => write!(f, "UDP {client} -> {external}"),
            NatKey::Ping { client, external } => write!(f, "Ping {client} -> {external}"),
        }
    }
}

#[async_trait]
pub trait NatHandler: Send {
    async fn receive(&self, packet: &[u8]) -> Result<()>;
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

#[async_trait]
pub trait NatHandlerFactory: Send {
    async fn nat(&self, key: NatKey) -> Option<Box<dyn NatHandler>>;
}

pub struct NatRouter {
    _mac: EthernetAddress,
    factory: Box<dyn NatHandlerFactory>,
    table: NatTable,
}

impl NatRouter {
    pub fn new(factory: Box<dyn NatHandlerFactory>, mac: EthernetAddress) -> Self {
        Self {
            _mac: mac,
            factory,
            table: NatTable::new(),
        }
    }

    pub async fn process(&mut self, data: &[u8]) -> Result<()> {
        let packet = SlicedPacket::from_ethernet(data)?;
        let Some(ref link) = packet.link else {
            return Ok(());
        };

        let LinkSlice::Ethernet2(ref ether) = link else {
            return Ok(());
        };

        let _mac = EthernetAddress(ether.destination());

        let Some(ref net) = packet.net else {
            return Ok(());
        };

        match net {
            NetSlice::Ipv4(ipv4) => {
                self.process_ipv4(data, ipv4).await?;
            }
            _ => {
                return Ok(());
            }
        }

        Ok(())
    }

    pub async fn process_ipv4<'a>(&mut self, data: &[u8], ipv4: &Ipv4Slice<'a>) -> Result<()> {
        let source_addr = IpAddress::Ipv4(ipv4.header().source_addr().into());
        let dest_addr = IpAddress::Ipv4(ipv4.header().destination_addr().into());

        match ipv4.header().protocol() {
            IpNumber::TCP => {
                self.process_tcp(data, source_addr, dest_addr, ipv4.payload())
                    .await?;
            }

            IpNumber::UDP => {
                self.process_udp(data, source_addr, dest_addr, ipv4.payload())
                    .await?;
            }

            _ => {}
        }

        Ok(())
    }

    pub async fn process_tcp<'a>(
        &mut self,
        data: &'a [u8],
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<()> {
        let header = TcpHeaderSlice::from_slice(payload.payload)?;
        let source = IpEndpoint::new(source_addr, header.source_port());
        let dest = IpEndpoint::new(dest_addr, header.destination_port());
        let key = NatKey::Tcp {
            client: source,
            external: dest,
        };
        self.process_nat(data, key).await?;
        Ok(())
    }

    pub async fn process_udp<'a>(
        &mut self,
        data: &'a [u8],
        source_addr: IpAddress,
        dest_addr: IpAddress,
        payload: &IpPayloadSlice<'a>,
    ) -> Result<()> {
        let header = UdpHeaderSlice::from_slice(payload.payload)?;
        let source = IpEndpoint::new(source_addr, header.source_port());
        let dest = IpEndpoint::new(dest_addr, header.destination_port());
        let key = NatKey::Udp {
            client: source,
            external: dest,
        };
        self.process_nat(data, key).await?;
        Ok(())
    }

    pub async fn process_nat(&mut self, data: &[u8], key: NatKey) -> Result<()> {
        let handler: Option<&mut Box<dyn NatHandler>> = match self.table.inner.entry(key) {
            Entry::Occupied(entry) => Some(entry.into_mut()),
            Entry::Vacant(entry) => {
                if let Some(handler) = self.factory.nat(key).await {
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
