use anyhow::Result;
use futures::TryStreamExt;
use log::{error, info, warn};
use netlink_packet_route::link::LinkAttribute;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;

use crate::backend::NetworkBackend;

mod backend;
mod raw_socket;

pub struct NetworkService {
    pub network: String,
}

impl NetworkService {
    pub fn new(network: String) -> Result<NetworkService> {
        Ok(NetworkService { network })
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
        let mut network = NetworkBackend::new(&self.network, &interface)?;
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
