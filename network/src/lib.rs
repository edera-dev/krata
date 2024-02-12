use std::time::Duration;

use anyhow::Result;
use autonet::{AutoNetworkChangeset, AutoNetworkCollector, NetworkMetadata};
use tokio::time::sleep;
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
            self.process_network_changeset(changeset)?;
            sleep(Duration::from_secs(2)).await;
        }
    }

    fn process_network_changeset(&mut self, changeset: AutoNetworkChangeset) -> Result<()> {
        for metadata in &changeset.added {
            futures::executor::block_on(async {
                self.add_network_backend(metadata.clone()).await
            })?;
        }

        Ok(())
    }

    async fn add_network_backend(&mut self, metadata: NetworkMetadata) -> Result<()> {
        let mut network = NetworkBackend::new(metadata, self.bridge.clone())?;
        network.init().await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        network.launch().await?;
        Ok(())
    }
}
