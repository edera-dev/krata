use super::{DeviceConfig, DeviceDescription, DeviceResult, XenTransaction};
use crate::error::{Error, Result};

pub struct VifDeviceConfig {
    backend_type: String,
    mac: Option<String>,
    mtu: Option<u32>,
    script: Option<String>,
    bridge: Option<String>,
    trusted: bool,
}

impl Default for VifDeviceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl VifDeviceConfig {
    pub fn new() -> Self {
        Self {
            backend_type: "vif".to_string(),
            mac: None,
            mtu: None,
            script: None,
            bridge: None,
            trusted: true,
        }
    }

    pub fn backend_type(&mut self, backend_type: impl AsRef<str>) -> &mut Self {
        self.backend_type = backend_type.as_ref().to_string();
        self
    }

    pub fn mac(&mut self, mac: impl AsRef<str>) -> &mut Self {
        self.mac = Some(mac.as_ref().to_string());
        self
    }

    pub fn mtu(&mut self, mtu: u32) -> &mut Self {
        self.mtu = Some(mtu);
        self
    }

    pub fn script(&mut self, script: impl AsRef<str>) -> &mut Self {
        self.script = Some(script.as_ref().to_string());
        self
    }

    pub fn bridge(&mut self, bridge: impl AsRef<str>) -> &mut Self {
        self.bridge = Some(bridge.as_ref().to_string());
        self
    }

    pub fn trusted(&mut self, trusted: bool) -> &mut Self {
        self.trusted = trusted;
        self
    }

    pub fn done(self) -> Self {
        self
    }
}

#[async_trait::async_trait]
impl DeviceConfig for VifDeviceConfig {
    type Result = DeviceResult;

    async fn add_to_transaction(&self, tx: &XenTransaction) -> Result<DeviceResult> {
        let id = tx.assign_next_devid().await?;
        let mac = self
            .mac
            .as_ref()
            .ok_or_else(|| Error::ParameterMissing("mac address"))?;
        let mtu = self
            .mtu
            .ok_or_else(|| Error::ParameterMissing("mtu"))?
            .to_string();
        let mut device = DeviceDescription::new("vif", &self.backend_type);
        device
            .add_backend_item("online", 1)
            .add_backend_item("state", 1)
            .add_backend_item("mac", mac)
            .add_backend_item("mtu", &mtu)
            .add_backend_item("type", "vif")
            .add_backend_item("handle", id);

        if let Some(bridge) = self.bridge.as_ref() {
            device.add_backend_item("bridge", bridge);
        }

        if let Some(script) = self.script.as_ref() {
            device
                .add_backend_item("script", script)
                .add_backend_item("hotplug-status", "");
        } else {
            device
                .add_backend_item("script", "")
                .add_backend_item("hotplug-status", "connected");
        }

        device
            .add_frontend_item("state", 1)
            .add_frontend_item("mac", mac)
            .add_frontend_item("mtu", &mtu)
            .add_frontend_bool("trusted", self.trusted);

        tx.add_device(id, device.done()).await?;
        Ok(DeviceResult { id })
    }
}
