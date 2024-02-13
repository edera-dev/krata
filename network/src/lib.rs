use std::time::Duration;

use anyhow::Result;
use autonet::{AutoNetworkChangeset, AutoNetworkCollector, NetworkMetadata};
use futures::{future::join_all, TryFutureExt};
use log::warn;
use tokio::time::sleep;
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
    pub bridge: VirtualBridge,
}

impl NetworkService {
    pub fn new() -> Result<NetworkService> {
        Ok(NetworkService {
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
        let futures = changeset
            .added
            .iter()
            .map(|metadata| {
                self.add_network_backend(metadata.clone())
                    .map_err(|x| (metadata.clone(), x))
            })
            .collect::<Vec<_>>();

        let failed = futures::executor::block_on(async move {
            let mut failed: Vec<Uuid> = Vec::new();
            let results = join_all(futures).await;
            for result in results {
                if let Err((metadata, error)) = result {
                    warn!(
                        "failed to launch network backend for hypha guest {}: {}",
                        metadata.uuid, error
                    );
                    failed.push(metadata.uuid);
                }
            }
            failed
        });

        for uuid in failed {
            collector.mark_unknown(uuid)?;
        }

        Ok(())
    }

    async fn add_network_backend(&self, metadata: NetworkMetadata) -> Result<()> {
        let mut network = NetworkBackend::new(metadata, self.bridge.clone())?;
        network.init().await?;
        network.launch().await?;
        Ok(())
    }
}
