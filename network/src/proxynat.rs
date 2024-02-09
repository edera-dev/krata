use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use etherparse::{SlicedPacket, UdpSlice};
use log::{debug, warn};
use smoltcp::{
    phy::{Checksum, ChecksumCapabilities},
    wire::{IpAddress, IpEndpoint},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc::channel,
};
use tokio::{sync::mpsc::Receiver, sync::mpsc::Sender};
use udp_stream::UdpStream;

use crate::nat::{NatHandler, NatHandlerFactory, NatKey};

pub struct ProxyNatHandlerFactory {}

struct ProxyUdpHandler {
    external: IpEndpoint,
    sender: Sender<Vec<u8>>,
}

impl ProxyNatHandlerFactory {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NatHandlerFactory for ProxyNatHandlerFactory {
    async fn nat(&self, key: NatKey) -> Option<Box<dyn NatHandler>> {
        debug!("creating proxy nat entry for key: {}", key);

        match key {
            NatKey::Udp {
                client: _,
                external,
            } => {
                let (sender, receiver) = channel::<Vec<u8>>(4);
                let mut handler = ProxyUdpHandler { external, sender };

                if let Err(error) = handler.spawn(receiver).await {
                    warn!("unable to spawn udp proxy handler: {}", error);
                    None
                } else {
                    Some(Box::new(handler))
                }
            }

            _ => None,
        }
    }
}

#[async_trait]
impl NatHandler for ProxyUdpHandler {
    async fn receive(&self, data: &[u8]) -> Result<()> {
        self.sender.try_send(data.to_vec())?;
        Ok(())
    }
}

enum ProxySelect {
    External(usize),
    Internal(Vec<u8>),
    Closed,
}

impl ProxyUdpHandler {
    async fn spawn(&mut self, receiver: Receiver<Vec<u8>>) -> Result<()> {
        let external_addr = match self.external.addr {
            IpAddress::Ipv4(addr) => SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(addr.0[0], addr.0[1], addr.0[2], addr.0[3])),
                self.external.port,
            ),
            IpAddress::Ipv6(_) => return Err(anyhow!("IPv6 unsupported")),
        };

        let socket = UdpStream::connect(external_addr).await?;
        tokio::spawn(async move {
            if let Err(error) = ProxyUdpHandler::process(socket, receiver).await {
                warn!("processing of udp proxy failed: {}", error);
            }
        });
        Ok(())
    }

    async fn process(mut socket: UdpStream, mut receiver: Receiver<Vec<u8>>) -> Result<()> {
        let mut checksum = ChecksumCapabilities::ignored();
        checksum.udp = Checksum::Tx;
        checksum.ipv4 = Checksum::Tx;
        checksum.tcp = Checksum::Tx;

        let mut external_buffer = vec![0u8; 2048];

        loop {
            let selection = select! {
                x = receiver.recv() => if let Some(data) = x {
                    ProxySelect::Internal(data)
                } else {
                    ProxySelect::Closed
                },
                x = socket.read(&mut external_buffer) => ProxySelect::External(x?),
            };

            match selection {
                ProxySelect::External(size) => {
                    let data = &external_buffer[0..size];
                    debug!("UDP from external: {:?}", data);
                }
                ProxySelect::Internal(data) => {
                    debug!("udp socket to handle data: {:?}", data);
                    let packet = SlicedPacket::from_ethernet(&data)?;
                    let Some(ref net) = packet.net else {
                        continue;
                    };

                    let Some(ip) = net.ip_payload_ref() else {
                        continue;
                    };

                    let udp = UdpSlice::from_slice(ip.payload)?;
                    debug!("UDP from internal: {:?}", udp.payload());
                    socket.write_all(udp.payload()).await?;
                }
                ProxySelect::Closed => warn!("UDP socket closed"),
            }
        }
    }
}
