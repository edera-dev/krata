use advmac::MacAddr6;
use anyhow::Result;
use futures::TryStreamExt;
use log::{error, info, warn};
use netlink_packet_route::link::LinkAttribute;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

use crate::backend::NetworkBackend;

pub mod backend;
pub mod chandev;
pub mod icmp;
pub mod nat;
pub mod proxynat;
pub mod raw_socket;

pub struct NetworkService {
    pub ipv4: String,
    pub ipv6: String,
    pub force_mac_address: Option<MacAddr6>,
}

impl NetworkService {
    pub fn new(
        ipv4: String,
        ipv6: String,
        force_mac_address: Option<MacAddr6>,
    ) -> Result<NetworkService> {
        Ok(NetworkService {
            ipv4,
            ipv6,
            force_mac_address,
        })
    }
}

impl NetworkService {
    pub async fn watch(&mut self) -> Result<()> {
        let spawned: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);
        loop {
            let mut stream = handle.link().get().execute();
            while let Some(message) = stream.try_next().await? {
                let mut name: Option<String> = None;
                for attribute in &message.attributes {
                    if let LinkAttribute::IfName(if_name) = attribute {
                        name = Some(if_name.clone());
                    }
                }

                if name.is_none() {
                    continue;
                }

                let name = name.unwrap();
                if !name.starts_with("vif") {
                    continue;
                }

                if let Ok(spawns) = spawned.lock() {
                    if spawns.contains(&name) {
                        continue;
                    }
                }

                if let Err(error) = self.add_network_backend(&name, spawned.clone()).await {
                    warn!(
                        "failed to initialize network backend for interface {}: {}",
                        name, error
                    );
                }

                if let Ok(mut spawns) = spawned.lock() {
                    spawns.push(name.clone());
                }
            }

            sleep(Duration::from_secs(2)).await;
        }
    }

    async fn add_network_backend(
        &mut self,
        interface: &str,
        spawned: Arc<Mutex<Vec<String>>>,
    ) -> Result<()> {
        let interface = interface.to_string();
        let mut network =
            NetworkBackend::new(&self.ipv4, &self.ipv6, &self.force_mac_address, &interface)?;
        info!("initializing network backend for interface {}", interface);
        network.init().await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        info!("spawning network backend for interface {}", interface);
        tokio::spawn(async move {
            if let Err(error) = network.run().await {
                error!(
                    "network backend for interface {} has been stopped: {}",
                    interface, error
                );
            }

            if let Ok(mut spawns) = spawned.lock() {
                if let Some(position) = spawns.iter().position(|x| *x == interface) {
                    spawns.remove(position);
                }
            }
        });
        Ok(())
    }
}
