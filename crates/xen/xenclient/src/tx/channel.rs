use xenplatform::domain::PlatformDomainInfo;

use super::{DeviceConfig, DeviceDescription, DeviceResult, XenTransaction};
use crate::error::{Error, Result};

pub struct ChannelDeviceConfig {
    backend_type: String,
    default_console: bool,
    default_console_options: Option<(u32, u64)>,
    backend_initialized: bool,
}

impl Default for ChannelDeviceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelDeviceConfig {
    pub fn new() -> Self {
        Self {
            backend_type: "console".to_string(),
            default_console: false,
            default_console_options: None,
            backend_initialized: false,
        }
    }

    pub fn backend_type(&mut self, backend_type: impl AsRef<str>) -> &mut Self {
        self.backend_type = backend_type.as_ref().to_string();
        self
    }

    pub fn default_console(&mut self) -> &mut Self {
        self.default_console = true;
        self
    }

    pub fn backend_initialized(&mut self) -> &mut Self {
        self.backend_initialized = true;
        self
    }

    pub fn done(self) -> Self {
        self
    }

    pub async fn prepare(&mut self, platform: &PlatformDomainInfo) -> Result<()> {
        if self.default_console {
            self.default_console_options = Some((platform.console_evtchn, platform.console_mfn));
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl DeviceConfig for ChannelDeviceConfig {
    type Result = DeviceResult;

    async fn add_to_transaction(&self, tx: &XenTransaction) -> Result<DeviceResult> {
        let id = tx.assign_next_devid().await?;
        let mut device = DeviceDescription::new("console", &self.backend_type);
        device
            .add_backend_bool("online", true)
            .add_backend_item("protocol", "vt100")
            .add_backend_item("type", &self.backend_type)
            .add_backend_item("state", if self.backend_initialized { 4 } else { 1 });

        if self.default_console {
            device.special_frontend_path("console");
            let (port, ring_ref) = self
                .default_console_options
                .as_ref()
                .ok_or_else(|| Error::ParameterMissing("default_console_options"))?;
            device
                .add_frontend_item("port", port)
                .add_frontend_item("ring-ref", ring_ref);
        }

        device
            .add_frontend_item("limit", 1048576)
            .add_frontend_item("output", "pty")
            .add_frontend_item("tty", "")
            .add_frontend_item("type", &self.backend_type)
            .add_frontend_item("state", 1);
        tx.add_device(id, device).await?;
        Ok(DeviceResult { id })
    }
}
