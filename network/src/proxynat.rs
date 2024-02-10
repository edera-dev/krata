use std::net::{IpAddr, Ipv4Addr, SocketAddr};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use etherparse::{PacketBuilder, SlicedPacket, UdpSlice};
use log::{debug, warn};
use smoltcp::{
    phy::{Checksum, ChecksumCapabilities},
    wire::IpAddress,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc::channel,
};
use tokio::{sync::mpsc::Receiver, sync::mpsc::Sender};
use udp_stream::UdpStream;

use crate::nat::{NatHandler, NatHandlerFactory, NatKey, NatKeyProtocol};

pub struct ProxyNatHandlerFactory {}

struct ProxyUdpHandler {
    key: NatKey,
    rx_sender: Sender<Vec<u8>>,
}

impl ProxyNatHandlerFactory {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NatHandlerFactory for ProxyNatHandlerFactory {
    async fn nat(&self, key: NatKey, sender: Sender<Vec<u8>>) -> Option<Box<dyn NatHandler>> {
        debug!("creating proxy nat entry for key: {}", key);

        match key.protocol {
            NatKeyProtocol::Udp => {
                let (rx_sender, rx_receiver) = channel::<Vec<u8>>(4);
                let mut handler = ProxyUdpHandler { key, rx_sender };

                if let Err(error) = handler.spawn(rx_receiver, sender.clone()).await {
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
        self.rx_sender.try_send(data.to_vec())?;
        Ok(())
    }
}

enum ProxySelect {
    External(usize),
    Internal(Vec<u8>),
    Closed,
}

impl ProxyUdpHandler {
    async fn spawn(
        &mut self,
        rx_receiver: Receiver<Vec<u8>>,
        tx_sender: Sender<Vec<u8>>,
    ) -> Result<()> {
        let external_addr = match self.key.external_ip.addr {
            IpAddress::Ipv4(addr) => SocketAddr::new(
                IpAddr::V4(Ipv4Addr::new(addr.0[0], addr.0[1], addr.0[2], addr.0[3])),
                self.key.external_ip.port,
            ),
            IpAddress::Ipv6(_) => return Err(anyhow!("IPv6 unsupported")),
        };

        let socket = UdpStream::connect(external_addr).await?;
        let key = self.key;
        tokio::spawn(async move {
            if let Err(error) = ProxyUdpHandler::process(key, socket, rx_receiver, tx_sender).await
            {
                warn!("processing of udp proxy failed: {}", error);
            }
        });
        Ok(())
    }

    async fn process(
        key: NatKey,
        mut socket: UdpStream,
        mut rx_receiver: Receiver<Vec<u8>>,
        tx_sender: Sender<Vec<u8>>,
    ) -> Result<()> {
        let mut checksum = ChecksumCapabilities::ignored();
        checksum.udp = Checksum::Tx;
        checksum.ipv4 = Checksum::Tx;
        checksum.tcp = Checksum::Tx;

        let mut external_buffer = vec![0u8; 2048];

        loop {
            let selection = select! {
                x = rx_receiver.recv() => if let Some(data) = x {
                    ProxySelect::Internal(data)
                } else {
                    ProxySelect::Closed
                },
                x = socket.read(&mut external_buffer) => ProxySelect::External(x?),
            };

            match selection {
                ProxySelect::External(size) => {
                    let data = &external_buffer[0..size];
                    let packet = PacketBuilder::ethernet2(key.local_mac.0, key.client_mac.0);
                    let packet = match (key.external_ip.addr, key.client_ip.addr) {
                        (IpAddress::Ipv4(external_addr), IpAddress::Ipv4(client_addr)) => {
                            packet.ipv4(external_addr.0, client_addr.0, 20)
                        }
                        (IpAddress::Ipv6(external_addr), IpAddress::Ipv6(client_addr)) => {
                            packet.ipv6(external_addr.0, client_addr.0, 20)
                        }
                        _ => {
                            return Err(anyhow!("IP endpoint mismatch"));
                        }
                    };
                    let packet = packet.udp(key.external_ip.port, key.client_ip.port);
                    let mut buffer: Vec<u8> = Vec::new();
                    packet.write(&mut buffer, data)?;
                    if let Err(error) = tx_sender.try_send(buffer) {
                        debug!("failed to transmit udp packet: {}", error);
                    }
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
