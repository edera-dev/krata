use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::{BufMut, BytesMut};
use etherparse::{PacketBuilder, SlicedPacket, UdpSlice};
use log::{debug, warn};
use smoltcp::wire::IpAddress;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
};
use tokio::{sync::mpsc::Receiver, sync::mpsc::Sender};
use udp_stream::UdpStream;

use crate::nat::handler::{NatHandler, NatHandlerContext};

const UDP_TIMEOUT_SECS: u64 = 60;

pub struct ProxyUdpHandler {
    rx_sender: Sender<BytesMut>,
}

#[async_trait]
impl NatHandler for ProxyUdpHandler {
    async fn receive(&self, data: &[u8]) -> Result<bool> {
        if self.rx_sender.is_closed() {
            Ok(true)
        } else {
            self.rx_sender.try_send(data.into())?;
            Ok(true)
        }
    }
}

enum ProxyUdpSelect {
    External(usize),
    Internal(BytesMut),
    Close,
}

impl ProxyUdpHandler {
    pub fn new(rx_sender: Sender<BytesMut>) -> Self {
        ProxyUdpHandler { rx_sender }
    }

    pub async fn spawn(
        &mut self,
        context: NatHandlerContext,
        rx_receiver: Receiver<BytesMut>,
    ) -> Result<()> {
        let external_addr = match context.key.external_ip.addr {
            IpAddress::Ipv4(addr) => {
                SocketAddr::new(IpAddr::V4(addr.0.into()), context.key.external_ip.port)
            }
            IpAddress::Ipv6(addr) => {
                SocketAddr::new(IpAddr::V6(addr.0.into()), context.key.external_ip.port)
            }
        };

        let socket = UdpStream::connect(external_addr).await?;
        tokio::spawn(async move {
            if let Err(error) = ProxyUdpHandler::process(context, socket, rx_receiver).await {
                warn!("processing of udp proxy failed: {}", error);
            }
        });
        Ok(())
    }

    async fn process(
        context: NatHandlerContext,
        mut socket: UdpStream,
        mut rx_receiver: Receiver<BytesMut>,
    ) -> Result<()> {
        let mut external_buffer = vec![0u8; 2048];

        loop {
            let deadline = tokio::time::sleep(Duration::from_secs(UDP_TIMEOUT_SECS));
            let selection = select! {
                x = rx_receiver.recv() => if let Some(data) = x {
                    ProxyUdpSelect::Internal(data)
                } else {
                    ProxyUdpSelect::Close
                },
                x = socket.read(&mut external_buffer) => ProxyUdpSelect::External(x?),
                _ = deadline => ProxyUdpSelect::Close,
            };

            match selection {
                ProxyUdpSelect::External(size) => {
                    let data = &external_buffer[0..size];
                    let packet =
                        PacketBuilder::ethernet2(context.key.local_mac.0, context.key.client_mac.0);
                    let packet = match (context.key.external_ip.addr, context.key.client_ip.addr) {
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
                    let packet =
                        packet.udp(context.key.external_ip.port, context.key.client_ip.port);
                    let buffer = BytesMut::with_capacity(packet.size(data.len()));
                    let mut writer = buffer.writer();
                    packet.write(&mut writer, data)?;
                    let buffer = writer.into_inner();
                    if let Err(error) = context.try_transmit(buffer) {
                        debug!("failed to transmit udp packet: {}", error);
                    }
                }
                ProxyUdpSelect::Internal(data) => {
                    let packet = SlicedPacket::from_ethernet(&data)?;
                    let Some(ref net) = packet.net else {
                        continue;
                    };

                    let Some(ip) = net.ip_payload_ref() else {
                        continue;
                    };

                    let udp = UdpSlice::from_slice(ip.payload)?;
                    socket.write_all(udp.payload()).await?;
                }
                ProxyUdpSelect::Close => {
                    drop(socket);
                    break;
                }
            }
        }

        context.reclaim().await?;

        Ok(())
    }
}
