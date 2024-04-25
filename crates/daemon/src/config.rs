use std::{collections::HashMap, path::Path};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::fs;

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DaemonConfig {
    #[serde(default)]
    pub pci: DaemonPciConfig,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct DaemonPciConfig {
    #[serde(default)]
    pub devices: HashMap<String, DaemonPciDeviceConfig>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct DaemonPciDeviceConfig {
    pub locations: Vec<String>,
    #[serde(default)]
    pub permissive: bool,
    #[serde(default)]
    #[serde(rename = "msi-translate")]
    pub msi_translate: bool,
    #[serde(default)]
    #[serde(rename = "power-management")]
    pub power_management: bool,
    #[serde(default)]
    #[serde(rename = "rdm-reserve-policy")]
    pub rdm_reserve_policy: DaemonPciDeviceRdmReservePolicy,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub enum DaemonPciDeviceRdmReservePolicy {
    #[default]
    #[serde(rename = "strict")]
    Strict,
    #[serde(rename = "relaxed")]
    Relaxed,
}

impl DaemonConfig {
    pub async fn load(path: &Path) -> Result<DaemonConfig> {
        if path.exists() {
            let content = fs::read_to_string(path).await?;
            let config: DaemonConfig = toml::from_str(&content)?;
            Ok(config)
        } else {
            fs::write(&path, "").await?;
            Ok(DaemonConfig::default())
        }
    }
}
