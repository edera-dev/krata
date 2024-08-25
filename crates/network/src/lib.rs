use std::{collections::HashMap, str::FromStr, time::Duration};

use anyhow::{anyhow, Result};
use autonet::{AutoNetworkChangeset, AutoNetworkWatcher, NetworkMetadata};
use futures::{future::join_all, TryFutureExt};
use hbridge::HostBridge;
use krata::{
    client::ControlClientProvider,
    dial::ControlDialAddress,
    v1::{
        common::Zone,
        control::{control_service_client::ControlServiceClient, GetHostStatusRequest},
    },
};
use log::warn;
use smoltcp::wire::{EthernetAddress, Ipv4Cidr, Ipv6Cidr};
use tokio::{task::JoinHandle, time::sleep};
use tonic::{transport::Channel, Request};
use uuid::Uuid;
use vbridge::VirtualBridge;

use crate::backend::NetworkBackend;

pub mod autonet;
pub mod backend;
pub mod chandev;
pub mod hbridge;
pub mod icmp;
pub mod nat;
pub mod pkt;
pub mod proxynat;
pub mod raw_socket;
pub mod vbridge;

const HOST_BRIDGE_MTU: usize = 1500;
pub const EXTRA_MTU: usize = 20;

pub struct NetworkService {
    pub control: ControlServiceClient<Channel>,
    pub zones: HashMap<Uuid, Zone>,
    pub backends: HashMap<Uuid, JoinHandle<()>>,
    pub bridge: VirtualBridge,
    pub hbridge: HostBridge,
}

impl NetworkService {
    pub async fn new(control_address: ControlDialAddress) -> Result<NetworkService> {
        let mut control = ControlClientProvider::dial(control_address).await?;
        let host_status = control
            .get_host_status(Request::new(GetHostStatusRequest {}))
            .await?
            .into_inner();
        let host_ipv4 = Ipv4Cidr::from_str(&host_status.host_ipv4)
            .map_err(|_| anyhow!("failed to parse host ipv4 cidr"))?;
        let host_ipv6 = Ipv6Cidr::from_str(&host_status.host_ipv6)
            .map_err(|_| anyhow!("failed to parse host ipv6 cidr"))?;
        let host_mac = EthernetAddress::from_str(&host_status.host_mac)
            .map_err(|_| anyhow!("failed to parse host mac address"))?;
        let bridge = VirtualBridge::new()?;
        let hbridge = HostBridge::new(
            HOST_BRIDGE_MTU + EXTRA_MTU,
            "krata0".to_string(),
            &bridge,
            host_ipv4,
            host_ipv6,
            host_mac,
        )
        .await?;
        Ok(NetworkService {
            control,
            zones: HashMap::new(),
            backends: HashMap::new(),
            bridge,
            hbridge,
        })
    }
}

impl NetworkService {
    pub async fn watch(&mut self) -> Result<()> {
        let mut watcher = AutoNetworkWatcher::new(self.control.clone()).await?;
        let mut receiver = watcher.events.subscribe();
        loop {
            let changeset = watcher.read_changes().await?;
            self.process_network_changeset(&mut watcher, changeset)
                .await?;
            watcher.wait(&mut receiver).await?;
        }
    }

    async fn process_network_changeset(
        &mut self,
        collector: &mut AutoNetworkWatcher,
        changeset: AutoNetworkChangeset,
    ) -> Result<()> {
        for removal in &changeset.removed {
            if let Some(handle) = self.backends.remove(&removal.uuid) {
                handle.abort();
            }
        }

        let futures = changeset
            .added
            .iter()
            .map(|metadata| {
                self.add_network_backend(metadata)
                    .map_err(|x| (metadata.clone(), x))
            })
            .collect::<Vec<_>>();

        sleep(Duration::from_secs(1)).await;
        let mut failed: Vec<Uuid> = Vec::new();
        let mut launched: Vec<(Uuid, JoinHandle<()>)> = Vec::new();
        let results = join_all(futures).await;
        for result in results {
            match result {
                Ok(launch) => {
                    launched.push(launch);
                }

                Err((metadata, error)) => {
                    warn!(
                        "failed to launch network backend for krata zone {}: {}",
                        metadata.uuid, error
                    );
                    failed.push(metadata.uuid);
                }
            };
        }

        for (uuid, handle) in launched {
            self.backends.insert(uuid, handle);
        }

        for uuid in failed {
            collector.mark_unknown(uuid)?;
        }

        Ok(())
    }

    async fn add_network_backend(
        &self,
        metadata: &NetworkMetadata,
    ) -> Result<(Uuid, JoinHandle<()>)> {
        let mut network = NetworkBackend::new(metadata.clone(), self.bridge.clone())?;
        network.init().await?;
        Ok((metadata.uuid, network.launch().await?))
    }
}
