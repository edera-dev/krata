use std::{net::IpAddr, time::Duration};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use etherparse::{
    Icmpv4Header, Icmpv4Type, Icmpv6Header, Icmpv6Type, IpNumber, Ipv4Slice, Ipv6Slice, NetSlice,
    PacketBuilder, SlicedPacket,
};
use log::{debug, warn};
use smoltcp::wire::IpAddress;
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender},
};

use crate::{
    icmp::{IcmpClient, IcmpProtocol, IcmpReply},
    nat::{NatHandler, NatHandlerContext},
};

const ICMP_PING_TIMEOUT_SECS: u64 = 20;
const ICMP_TIMEOUT_SECS: u64 = 30;

pub struct ProxyIcmpHandler {
    rx_sender: Sender<Vec<u8>>,
}

#[async_trait]
impl NatHandler for ProxyIcmpHandler {
    async fn receive(&self, data: &[u8]) -> Result<()> {
        self.rx_sender.try_send(data.to_vec())?;
        Ok(())
    }
}

enum ProxyIcmpSelect {
    Internal(Vec<u8>),
    Close,
}

impl ProxyIcmpHandler {
    pub fn new(rx_sender: Sender<Vec<u8>>) -> Self {
        ProxyIcmpHandler { rx_sender }
    }

    pub async fn spawn(
        &mut self,
        context: NatHandlerContext,
        rx_receiver: Receiver<Vec<u8>>,
    ) -> Result<()> {
        let client = IcmpClient::new(match context.key.external_ip.addr {
            IpAddress::Ipv4(_) => IcmpProtocol::Icmpv4,
            IpAddress::Ipv6(_) => IcmpProtocol::Icmpv6,
        })?;
        tokio::spawn(async move {
            if let Err(error) = ProxyIcmpHandler::process(client, rx_receiver, context).await {
                warn!("processing of icmp proxy failed: {}", error);
            }
        });
        Ok(())
    }

    async fn process(
        client: IcmpClient,
        mut rx_receiver: Receiver<Vec<u8>>,
        context: NatHandlerContext,
    ) -> Result<()> {
        loop {
            let deadline = tokio::time::sleep(Duration::from_secs(ICMP_TIMEOUT_SECS));
            let selection = select! {
                x = rx_receiver.recv() => if let Some(data) = x {
                    ProxyIcmpSelect::Internal(data)
                } else {
                    ProxyIcmpSelect::Close
                },
                _ =  deadline => ProxyIcmpSelect::Close,
            };

            match selection {
                ProxyIcmpSelect::Internal(data) => {
                    let packet = SlicedPacket::from_ethernet(&data)?;
                    let Some(ref net) = packet.net else {
                        continue;
                    };

                    match net {
                        NetSlice::Ipv4(ipv4) => {
                            ProxyIcmpHandler::process_ipv4(&context, ipv4, &client).await?
                        }

                        NetSlice::Ipv6(ipv6) => {
                            ProxyIcmpHandler::process_ipv6(&context, ipv6, &client).await?
                        }
                    }
                }

                ProxyIcmpSelect::Close => {
                    break;
                }
            }
        }

        Ok(())
    }

    async fn process_ipv4(
        context: &NatHandlerContext,
        ipv4: &Ipv4Slice<'_>,
        client: &IcmpClient,
    ) -> Result<()> {
        if ipv4.header().protocol() != IpNumber::ICMP {
            return Ok(());
        }

        let (header, payload) = Icmpv4Header::from_slice(ipv4.payload().payload)?;
        if let Icmpv4Type::EchoRequest(echo) = header.icmp_type {
            let IpAddr::V4(external_ipv4) = context.key.external_ip.addr.into() else {
                return Ok(());
            };

            let Some(IcmpReply::Icmp4 {
                header: _,
                echo,
                payload,
            }) = client
                .ping4(
                    external_ipv4,
                    echo.id,
                    echo.seq,
                    payload,
                    Duration::from_secs(ICMP_PING_TIMEOUT_SECS),
                )
                .await?
            else {
                return Ok(());
            };

            let packet =
                PacketBuilder::ethernet2(context.key.local_mac.0, context.key.client_mac.0);
            let packet = match (context.key.external_ip.addr, context.key.client_ip.addr) {
                (IpAddress::Ipv4(external_addr), IpAddress::Ipv4(client_addr)) => {
                    packet.ipv4(external_addr.0, client_addr.0, 20)
                }
                _ => {
                    return Err(anyhow!("IP endpoint mismatch"));
                }
            };
            let packet = packet.icmpv4_echo_reply(echo.id, echo.seq);
            let mut buffer: Vec<u8> = Vec::new();
            packet.write(&mut buffer, &payload)?;
            if let Err(error) = context.try_send(buffer) {
                debug!("failed to transmit icmp packet: {}", error);
            }
        }
        Ok(())
    }

    async fn process_ipv6(
        context: &NatHandlerContext,
        ipv6: &Ipv6Slice<'_>,
        client: &IcmpClient,
    ) -> Result<()> {
        if ipv6.header().next_header() != IpNumber::IPV6_ICMP {
            return Ok(());
        }

        let (header, payload) = Icmpv6Header::from_slice(ipv6.payload().payload)?;
        if let Icmpv6Type::EchoRequest(echo) = header.icmp_type {
            let IpAddr::V6(external_ipv6) = context.key.external_ip.addr.into() else {
                return Ok(());
            };

            let Some(IcmpReply::Icmp6 {
                header: _,
                echo,
                payload,
            }) = client
                .ping6(
                    external_ipv6,
                    echo.id,
                    echo.seq,
                    payload,
                    Duration::from_secs(ICMP_PING_TIMEOUT_SECS),
                )
                .await?
            else {
                return Ok(());
            };

            let packet =
                PacketBuilder::ethernet2(context.key.local_mac.0, context.key.client_mac.0);
            let packet = match (context.key.external_ip.addr, context.key.client_ip.addr) {
                (IpAddress::Ipv6(external_addr), IpAddress::Ipv6(client_addr)) => {
                    packet.ipv6(external_addr.0, client_addr.0, 20)
                }
                _ => {
                    return Err(anyhow!("IP endpoint mismatch"));
                }
            };
            let packet = packet.icmpv6_echo_reply(echo.id, echo.seq);
            let mut buffer: Vec<u8> = Vec::new();
            packet.write(&mut buffer, &payload)?;
            if let Err(error) = context.try_send(buffer) {
                debug!("failed to transmit icmp packet: {}", error);
            }
        }

        Ok(())
    }
}
