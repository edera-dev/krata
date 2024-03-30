use std::{
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    time::Duration,
};

use anyhow::{anyhow, Result};
use async_trait::async_trait;
use bytes::{BufMut, BytesMut};
use etherparse::{
    IcmpEchoHeader, Icmpv4Header, Icmpv4Type, Icmpv6Header, Icmpv6Type, IpNumber, Ipv4Slice,
    Ipv6Slice, NetSlice, PacketBuilder, SlicedPacket,
};
use log::{debug, trace, warn};
use smoltcp::wire::IpAddress;
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender},
};

use crate::{
    icmp::{IcmpClient, IcmpProtocol, IcmpReply},
    nat::handler::{NatHandler, NatHandlerContext},
};

const ICMP_PING_TIMEOUT_SECS: u64 = 20;
const ICMP_TIMEOUT_SECS: u64 = 30;

pub struct ProxyIcmpHandler {
    rx_sender: Sender<BytesMut>,
}

#[async_trait]
impl NatHandler for ProxyIcmpHandler {
    async fn receive(&self, data: &[u8]) -> Result<bool> {
        if self.rx_sender.is_closed() {
            Ok(true)
        } else {
            self.rx_sender.try_send(data.into())?;
            Ok(true)
        }
    }
}

enum ProxyIcmpSelect {
    Internal(BytesMut),
    Close,
}

impl ProxyIcmpHandler {
    pub fn new(rx_sender: Sender<BytesMut>) -> Self {
        ProxyIcmpHandler { rx_sender }
    }

    pub async fn spawn(
        &mut self,
        context: NatHandlerContext,
        rx_receiver: Receiver<BytesMut>,
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
        mut rx_receiver: Receiver<BytesMut>,
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

        context.reclaim().await?;

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

            let context = context.clone();
            let client = client.clone();
            let payload = payload.to_vec();
            tokio::task::spawn(async move {
                if let Err(error) = ProxyIcmpHandler::process_echo_ipv4(
                    context,
                    client,
                    external_ipv4,
                    echo,
                    payload,
                )
                .await
                {
                    trace!("icmp4 echo failed: {}", error);
                }
            });
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

            let context = context.clone();
            let client = client.clone();
            let payload = payload.to_vec();
            tokio::task::spawn(async move {
                if let Err(error) = ProxyIcmpHandler::process_echo_ipv6(
                    context,
                    client,
                    external_ipv6,
                    echo,
                    payload,
                )
                .await
                {
                    trace!("icmp6 echo failed: {}", error);
                }
            });
        }

        Ok(())
    }

    async fn process_echo_ipv4(
        context: NatHandlerContext,
        client: IcmpClient,
        external_ipv4: Ipv4Addr,
        echo: IcmpEchoHeader,
        payload: Vec<u8>,
    ) -> Result<()> {
        let reply = client
            .ping4(
                external_ipv4,
                echo.id,
                echo.seq,
                &payload,
                Duration::from_secs(ICMP_PING_TIMEOUT_SECS),
            )
            .await?;
        let Some(IcmpReply::Icmpv4 {
            header: _,
            echo,
            payload,
        }) = reply
        else {
            return Ok(());
        };

        let packet = PacketBuilder::ethernet2(context.key.local_mac.0, context.key.client_mac.0);
        let packet = match (context.key.external_ip.addr, context.key.client_ip.addr) {
            (IpAddress::Ipv4(external_addr), IpAddress::Ipv4(client_addr)) => {
                packet.ipv4(external_addr.0, client_addr.0, 20)
            }
            _ => {
                return Err(anyhow!("IP endpoint mismatch"));
            }
        };
        let packet = packet.icmpv4_echo_reply(echo.id, echo.seq);
        let buffer = BytesMut::with_capacity(packet.size(payload.len()));
        let mut writer = buffer.writer();
        packet.write(&mut writer, &payload)?;
        let buffer = writer.into_inner();
        if let Err(error) = context.try_transmit(buffer) {
            debug!("failed to transmit icmp packet: {}", error);
        }
        Ok(())
    }

    async fn process_echo_ipv6(
        context: NatHandlerContext,
        client: IcmpClient,
        external_ipv6: Ipv6Addr,
        echo: IcmpEchoHeader,
        payload: Vec<u8>,
    ) -> Result<()> {
        let reply = client
            .ping6(
                external_ipv6,
                echo.id,
                echo.seq,
                &payload,
                Duration::from_secs(ICMP_PING_TIMEOUT_SECS),
            )
            .await?;
        let Some(IcmpReply::Icmpv6 {
            header: _,
            echo,
            payload,
        }) = reply
        else {
            return Ok(());
        };

        let packet = PacketBuilder::ethernet2(context.key.local_mac.0, context.key.client_mac.0);
        let packet = match (context.key.external_ip.addr, context.key.client_ip.addr) {
            (IpAddress::Ipv6(external_addr), IpAddress::Ipv6(client_addr)) => {
                packet.ipv6(external_addr.0, client_addr.0, 20)
            }
            _ => {
                return Err(anyhow!("IP endpoint mismatch"));
            }
        };
        let packet = packet.icmpv6_echo_reply(echo.id, echo.seq);
        let buffer = BytesMut::with_capacity(packet.size(payload.len()));
        let mut writer = buffer.writer();
        packet.write(&mut writer, &payload)?;
        let buffer = writer.into_inner();
        if let Err(error) = context.try_transmit(buffer) {
            debug!("failed to transmit icmp packet: {}", error);
        }
        Ok(())
    }
}
