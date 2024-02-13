use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use autonet::{AutoNetworkChangeset, AutoNetworkCollector, NetworkMetadata};
use futures::{future::join_all, TryFutureExt};
use log::warn;
use tokio::{task::JoinHandle, time::sleep};
use uuid::Uuid;
use vbridge::VirtualBridge;

use crate::backend::NetworkBackend;

pub mod autonet;
pub mod backend;
pub mod chandev;
pub mod icmp;
pub mod nat;
pub mod pkt;
pub mod proxynat;
pub mod raw_socket;
pub mod vbridge;

pub struct NetworkService {
    pub backends: HashMap<Uuid, JoinHandle<()>>,
    pub bridge: VirtualBridge,
}

impl NetworkService {
    pub fn new() -> Result<NetworkService> {
        Ok(NetworkService {
            backends: HashMap::new(),
            bridge: VirtualBridge::new()?,
        })
    }
}

impl NetworkService {
    pub async fn watch(&mut self) -> Result<()> {
        let mut collector = AutoNetworkCollector::new()?;
        loop {
            let changeset = collector.read_changes()?;
            self.process_network_changeset(&mut collector, changeset)?;
            sleep(Duration::from_secs(2)).await;
        }
    }

    fn process_network_changeset(
        &mut self,
        collector: &mut AutoNetworkCollector,
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

        let (launched, failed) = futures::executor::block_on(async move {
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
                            "failed to launch network backend for hypha guest {}: {}",
                            metadata.uuid, error
                        );
                        failed.push(metadata.uuid);
                    }
                };
            }
            (launched, failed)
        });

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
