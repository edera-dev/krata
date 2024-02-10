use std::time::Duration;

use anyhow::{anyhow, Result};
use async_ping::{
    icmp_client::Config,
    icmp_packet::{Icmp, Icmpv4},
    PingClient,
};
use async_trait::async_trait;
use etherparse::{Icmpv4Header, Icmpv4Type, IpNumber, PacketBuilder, SlicedPacket};
use log::{debug, warn};
use smoltcp::wire::IpAddress;
use tokio::{
    select,
    sync::mpsc::{Receiver, Sender},
};

use crate::nat::{NatHandler, NatKey};

const ICMP_PING_TIMEOUT_SECS: u64 = 20;
const ICMP_TIMEOUT_SECS: u64 = 30;

pub struct ProxyIcmpHandler {
    key: NatKey,
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
    pub fn new(key: NatKey, rx_sender: Sender<Vec<u8>>) -> Self {
        ProxyIcmpHandler { key, rx_sender }
    }

    pub async fn spawn(
        &mut self,
        rx_receiver: Receiver<Vec<u8>>,
        tx_sender: Sender<Vec<u8>>,
        reclaim_sender: Sender<NatKey>,
    ) -> Result<()> {
        let client = PingClient::<icmp_client::impl_tokio::Client>::new(Some(Config::new()), None)?;

        {
            let client = client.clone();
            tokio::spawn(async move {
                client.handle_v4_recv_from().await;
            });
        }

        let key = self.key;
        tokio::spawn(async move {
            if let Err(error) =
                ProxyIcmpHandler::process(client, key, rx_receiver, tx_sender, reclaim_sender).await
            {
                warn!("processing of icmp proxy failed: {}", error);
            }
        });
        Ok(())
    }

    async fn process(
        client: PingClient<icmp_client::impl_tokio::Client>,
        key: NatKey,
        mut rx_receiver: Receiver<Vec<u8>>,
        tx_sender: Sender<Vec<u8>>,
        reclaim_sender: Sender<NatKey>,
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

                    let Some(ip) = net.ip_payload_ref() else {
                        continue;
                    };

                    if ip.ip_number != IpNumber::ICMP {
                        continue;
                    }

                    let (header, payload) = Icmpv4Header::from_slice(ip.payload)?;
                    if let Icmpv4Type::EchoRequest(echo) = header.icmp_type {
                        let result = client
                            .ping(
                                key.external_ip.addr.into(),
                                Some(echo.id),
                                Some(echo.seq),
                                payload,
                                Duration::from_secs(ICMP_PING_TIMEOUT_SECS),
                            )
                            .await;
                        match result {
                            Ok((icmp, _)) => match icmp {
                                Icmp::V4(Icmpv4::EchoReply(reply)) => {
                                    let packet =
                                        PacketBuilder::ethernet2(key.local_mac.0, key.client_mac.0);
                                    let packet = match (key.external_ip.addr, key.client_ip.addr) {
                                        (
                                            IpAddress::Ipv4(external_addr),
                                            IpAddress::Ipv4(client_addr),
                                        ) => packet.ipv4(external_addr.0, client_addr.0, 20),
                                        (
                                            IpAddress::Ipv6(external_addr),
                                            IpAddress::Ipv6(client_addr),
                                        ) => packet.ipv6(external_addr.0, client_addr.0, 20),
                                        _ => {
                                            return Err(anyhow!("IP endpoint mismatch"));
                                        }
                                    };
                                    let packet = packet.icmpv4_echo_reply(
                                        reply.identifier.0,
                                        reply.sequence_number.0,
                                    );
                                    let mut buffer: Vec<u8> = Vec::new();
                                    packet.write(&mut buffer, &reply.payload)?;
                                    if let Err(error) = tx_sender.try_send(buffer) {
                                        debug!("failed to transmit icmp packet: {}", error);
                                    }
                                }

                                Icmp::V4(Icmpv4::Other(_type, _code, _payload)) => {}

                                _ => {}
                            },

                            Err(error) => {
                                debug!("proxy for icmp failed to emulate ICMP ping: {}", error);
                            }
                        }
                    }
                }

                ProxyIcmpSelect::Close => {
                    reclaim_sender.send(key).await?;
                    break;
                }
            }
        }

        Ok(())
    }
}
