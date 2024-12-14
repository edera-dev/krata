use super::{DeviceConfig, DeviceDescription, DeviceResult, XenTransaction};
use crate::error::{Error, Result};

pub struct Fs9pDeviceConfig {
    backend_type: String,
    security_model: String,
    path: Option<String>,
    tag: Option<String>,
}

impl Default for Fs9pDeviceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl Fs9pDeviceConfig {
    pub fn new() -> Self {
        Self {
            backend_type: "9pfs".to_string(),
            security_model: "none".to_string(),
            path: None,
            tag: None,
        }
    }

    pub fn backend_type(&mut self, backend_type: impl AsRef<str>) -> &mut Self {
        self.backend_type = backend_type.as_ref().to_string();
        self
    }

    pub fn security_model(&mut self, security_model: impl AsRef<str>) -> &mut Self {
        self.security_model = security_model.as_ref().to_string();
        self
    }

    pub fn path(&mut self, path: impl AsRef<str>) -> &mut Self {
        self.path = Some(path.as_ref().to_string());
        self
    }

    pub fn tag(&mut self, tag: impl AsRef<str>) -> &mut Self {
        self.tag = Some(tag.as_ref().to_string());
        self
    }

    pub fn done(self) -> Self {
        self
    }
}

#[async_trait::async_trait]
impl DeviceConfig for Fs9pDeviceConfig {
    type Result = DeviceResult;

    async fn add_to_transaction(&self, tx: &XenTransaction) -> Result<DeviceResult> {
        let id = tx.assign_next_devid().await?;
        let path = self
            .path
            .as_ref()
            .ok_or_else(|| Error::ParameterMissing("path"))?;
        let tag = self
            .tag
            .as_ref()
            .ok_or_else(|| Error::ParameterMissing("tag"))?;
        let mut device = DeviceDescription::new("9pfs", &self.backend_type);
        device
            .add_backend_bool("online", true)
            .add_backend_item("state", 1)
            .add_backend_item("path", path)
            .add_backend_item("security_model", &self.security_model);
        device
            .add_frontend_item("state", 1)
            .add_frontend_item("tag", tag);
        tx.add_device(id, device).await?;
        Ok(DeviceResult { id })
    }
}
