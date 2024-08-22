use std::{io::ErrorKind, net::IpAddr};

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use futures::TryStreamExt;
use log::error;
use smoltcp::wire::{EthernetAddress, Ipv4Cidr, Ipv6Cidr};
use tokio::{select, task::JoinHandle};
use tokio_tun::Tun;

use crate::vbridge::{BridgeJoinHandle, VirtualBridge};

#[derive(Debug)]
enum HostBridgeProcessSelect {
    Send(Option<BytesMut>),
    Receive(std::io::Result<usize>),
}

pub struct HostBridge {
    task: JoinHandle<()>,
}

impl HostBridge {
    pub async fn new(
        mtu: usize,
        interface: String,
        bridge: &VirtualBridge,
        ipv4: Ipv4Cidr,
        ipv6: Ipv6Cidr,
        mac: EthernetAddress,
    ) -> Result<HostBridge> {
        let tun = Tun::builder()
            .name(&interface)
            .tap(true)
            .mtu(mtu as i32)
            .packet_info(false)
            .try_build()?;

        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle.link().get().match_name(interface.clone()).execute();
        let link = links.try_next().await?;
        if link.is_none() {
            return Err(anyhow!(
                "unable to find network interface named {}",
                interface
            ));
        }
        let link = link.unwrap();

        handle
            .address()
            .add(
                link.header.index,
                IpAddr::V4(ipv4.address().into()),
                ipv4.prefix_len(),
            )
            .execute()
            .await?;

        handle
            .address()
            .add(
                link.header.index,
                IpAddr::V6(ipv6.address().into()),
                ipv6.prefix_len(),
            )
            .execute()
            .await?;

        handle
            .link()
            .set(link.header.index)
            .address(mac.0.to_vec())
            .up()
            .execute()
            .await?;

        let bridge_handle = bridge.join(mac).await?;

        let task = tokio::task::spawn(async move {
            if let Err(error) = HostBridge::process(mtu, tun, bridge_handle).await {
                error!("failed to process host bridge: {}", error);
            }
        });

        Ok(HostBridge { task })
    }

    async fn process(mtu: usize, tun: Tun, mut bridge_handle: BridgeJoinHandle) -> Result<()> {
        let tear_off_size = 100 * mtu;
        let mut buffer: BytesMut = BytesMut::with_capacity(tear_off_size);
        loop {
            if buffer.capacity() < mtu {
                buffer = BytesMut::with_capacity(tear_off_size);
            }

            buffer.resize(mtu, 0);
            let selection = select! {
                biased;
                x = tun.recv(&mut buffer) => HostBridgeProcessSelect::Receive(x),
                x = bridge_handle.from_bridge_receiver.recv() => HostBridgeProcessSelect::Send(x),
                x = bridge_handle.from_broadcast_receiver.recv() => HostBridgeProcessSelect::Send(x.ok()),
            };

            match selection {
                HostBridgeProcessSelect::Send(Some(bytes)) => match tun.try_send(&bytes) {
                    Ok(_) => {}
                    Err(error) => {
                        if error.kind() == ErrorKind::WouldBlock {
                            continue;
                        }
                        return Err(error.into());
                    }
                },

                HostBridgeProcessSelect::Send(None) => {
                    break;
                }

                HostBridgeProcessSelect::Receive(result) => match result {
                    Ok(len) => {
                        if len == 0 {
                            continue;
                        }
                        let packet = buffer.split_to(len);
                        let _ = bridge_handle.to_bridge_sender.try_send(packet);
                    }

                    Err(error) => {
                        if error.kind() == ErrorKind::WouldBlock {
                            continue;
                        }

                        error!(
                            "failed to receive data from tap device to bridge: {}",
                            error
                        );
                        break;
                    }
                },
            }
        }
        Ok(())
    }
}

impl Drop for HostBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}
