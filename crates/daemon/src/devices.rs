use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, Result};
use log::warn;
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::config::{DaemonConfig, DaemonPciDeviceConfig};

#[derive(Clone)]
pub struct DaemonDeviceState {
    pub pci: Option<DaemonPciDeviceConfig>,
    pub owner: Option<Uuid>,
}

#[derive(Clone)]
pub struct DaemonDeviceManager {
    config: Arc<DaemonConfig>,
    devices: Arc<RwLock<HashMap<String, DaemonDeviceState>>>,
}

impl DaemonDeviceManager {
    pub fn new(config: Arc<DaemonConfig>) -> Self {
        Self {
            config,
            devices: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub async fn claim(&self, device: &str, uuid: Uuid) -> Result<DaemonDeviceState> {
        let mut devices = self.devices.write().await;
        let Some(state) = devices.get_mut(device) else {
            return Err(anyhow!(
                "unable to claim unknown device '{}' for guest {}",
                device,
                uuid
            ));
        };

        if let Some(owner) = state.owner {
            return Err(anyhow!(
                "unable to claim device '{}' for guest {}: already claimed by {}",
                device,
                uuid,
                owner
            ));
        }

        state.owner = Some(uuid);
        Ok(state.clone())
    }

    pub async fn release_all(&self, uuid: Uuid) -> Result<()> {
        let mut devices = self.devices.write().await;
        for state in (*devices).values_mut() {
            if state.owner == Some(uuid) {
                state.owner = None;
            }
        }
        Ok(())
    }

    pub async fn release(&self, device: &str, uuid: Uuid) -> Result<()> {
        let mut devices = self.devices.write().await;
        let Some(state) = devices.get_mut(device) else {
            return Ok(());
        };

        if let Some(owner) = state.owner {
            if owner != uuid {
                return Ok(());
            }
        }

        state.owner = None;
        Ok(())
    }

    pub async fn update_claims(&self, claims: HashMap<String, Uuid>) -> Result<()> {
        let mut devices = self.devices.write().await;
        devices.clear();
        for (name, pci) in &self.config.pci.devices {
            let owner = claims.get(name).cloned();
            devices.insert(
                name.clone(),
                DaemonDeviceState {
                    owner,
                    pci: Some(pci.clone()),
                },
            );
        }

        for (name, uuid) in &claims {
            if !devices.contains_key(name) {
                warn!("unknown device '{}' assigned to guest {}", name, uuid);
            }
        }

        Ok(())
    }

    pub async fn copy(&self) -> Result<HashMap<String, DaemonDeviceState>> {
        let devices = self.devices.read().await;
        Ok(devices.clone())
    }
}
