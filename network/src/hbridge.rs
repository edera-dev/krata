use std::net::{IpAddr, Ipv4Addr};

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use futures::TryStreamExt;
use log::error;
use smoltcp::wire::EthernetAddress;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc::channel,
    task::JoinHandle,
};
use tokio_tun::Tun;

use crate::vbridge::{BridgeJoinHandle, VirtualBridge};

pub struct HostBridge {
    task: JoinHandle<()>,
}

impl HostBridge {
    pub async fn new(interface: String, bridge: &VirtualBridge) -> Result<HostBridge> {
        let tun = Tun::builder()
            .name(&interface)
            .tap(true)
            .mtu(1500)
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
            .add(
                link.header.index,
                IpAddr::V4(Ipv4Addr::new(10, 75, 0, 1)),
                16,
            )
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
            if let Err(error) = HostBridge::process(tun, bridge_handle).await {
                error!("failed to process host bridge: {}", error);
            }
        });

        Ok(HostBridge { task })
    }

    async fn process(tun: Tun, mut bridge_handle: BridgeJoinHandle) -> Result<()> {
        let (rx_sender, mut rx_receiver) = channel::<BytesMut>(100);
        let (mut read, mut write) = tokio::io::split(tun);
        tokio::task::spawn(async move {
            let mut buffer = vec![0u8; 1500];
            loop {
                let size = match read.read(&mut buffer).await {
                    Ok(size) => size,
                    Err(error) => {
                        error!("failed to read tap device: {}", error);
                        break;
                    }
                };
                match rx_sender.send(buffer[0..size].into()).await {
                    Ok(_) => {}
                    Err(error) => {
                        error!(
                            "failed to send data from tap device to processor: {}",
                            error
                        );
                        break;
                    }
                }
            }
        });
        loop {
            select! {
                x = bridge_handle.from_bridge_receiver.recv() => match x {
                    Some(bytes) => {
                        write.write_all(&bytes).await?;
                    },
                    None => {
                        break;
                    }
                },
                x = bridge_handle.from_broadcast_receiver.recv() => match x {
                    Ok(bytes) => {
                        write.write_all(&bytes).await?;
                    },
                    Err(error) => {
                        return Err(error.into());
                    }
                },
                x = rx_receiver.recv() => match x {
                    Some(bytes) => {
                        bridge_handle.to_bridge_sender.send(bytes).await?;
                    },
                    None => {
                        break;
                    }
                }
            };
        }
        Ok(())
    }
}

impl Drop for HostBridge {
    fn drop(&mut self) {
        self.task.abort();
    }
}
