use std::{
    io::ErrorKind,
    net::{IpAddr, Ipv4Addr},
};

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use futures::TryStreamExt;
use log::error;
use smoltcp::wire::EthernetAddress;
use tokio::{select, task::JoinHandle};
use tokio_tun::Tun;

use crate::vbridge::{BridgeJoinHandle, VirtualBridge};

const HOST_IPV4_ADDR: Ipv4Addr = Ipv4Addr::new(10, 75, 0, 1);

#[derive(Debug)]
enum HostBridgeProcessSelect {
    Send(Option<BytesMut>),
    Receive(std::io::Result<usize>),
}

pub struct HostBridge {
    task: JoinHandle<()>,
}

impl HostBridge {
    pub async fn new(mtu: usize, interface: String, bridge: &VirtualBridge) -> Result<HostBridge> {
        let tun = Tun::builder()
            .name(&interface)
            .tap(true)
            .mtu(mtu as i32)
            .packet_info(false)
            .try_build()?;

        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut mac = MacAddr6::random();
        mac.set_local(true);
        mac.set_multicast(false);

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
            .add(link.header.index, IpAddr::V4(HOST_IPV4_ADDR), 16)
            .execute()
            .await?;

        handle
            .address()
            .add(link.header.index, IpAddr::V6(mac.to_link_local_ipv6()), 10)
            .execute()
            .await?;

        handle
            .link()
            .set(link.header.index)
            .address(mac.to_array().to_vec())
            .up()
            .execute()
            .await?;

        let mac = EthernetAddress(mac.to_array());
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
