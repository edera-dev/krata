use std::collections::HashMap;

use xencall::XenCall;
pub use xenplatform::domain::PlatformDomainConfig;
use xenplatform::domain::PlatformDomainInfo;

use crate::{
    error::Result,
    tx::{
        channel::ChannelDeviceConfig,
        fs9p::Fs9pDeviceConfig,
        pci::PciRootDeviceConfig,
        vbd::VbdDeviceConfig,
        vif::VifDeviceConfig,
        {BlockDeviceResult, DeviceResult},
    },
};

pub struct DomainConfig {
    platform: Option<PlatformDomainConfig>,
    name: Option<String>,
    backend_domid: u32,
    channels: Vec<ChannelDeviceConfig>,
    vifs: Vec<VifDeviceConfig>,
    vbds: Vec<VbdDeviceConfig>,
    fs9ps: Vec<Fs9pDeviceConfig>,
    pci: Option<PciRootDeviceConfig>,
    extra_keys: HashMap<String, String>,
    extra_rw_paths: Vec<String>,
    start: bool,
}

impl Default for DomainConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl DomainConfig {
    pub fn new() -> Self {
        Self {
            platform: None,
            name: None,
            backend_domid: 0,
            channels: Vec::new(),
            vifs: Vec::new(),
            vbds: Vec::new(),
            fs9ps: Vec::new(),
            pci: None,
            extra_keys: HashMap::new(),
            extra_rw_paths: Vec::new(),
            start: true,
        }
    }

    pub fn platform(&mut self, platform: PlatformDomainConfig) -> &mut Self {
        self.platform = Some(platform);
        self
    }

    pub fn get_platform(&self) -> &Option<PlatformDomainConfig> {
        &self.platform
    }

    pub fn name(&mut self, name: impl AsRef<str>) -> &mut Self {
        self.name = Some(name.as_ref().to_string());
        self
    }

    pub fn get_name(&self) -> &Option<String> {
        &self.name
    }

    pub fn backend_domid(&mut self, backend_domid: u32) -> &mut Self {
        self.backend_domid = backend_domid;
        self
    }

    pub fn get_backend_domid(&self) -> u32 {
        self.backend_domid
    }

    pub fn add_channel(&mut self, channel: ChannelDeviceConfig) -> &mut Self {
        self.channels.push(channel);
        self
    }

    pub fn get_channels(&self) -> &Vec<ChannelDeviceConfig> {
        &self.channels
    }

    pub fn add_vif(&mut self, vif: VifDeviceConfig) -> &mut Self {
        self.vifs.push(vif);
        self
    }

    pub fn get_vifs(&self) -> &Vec<VifDeviceConfig> {
        &self.vifs
    }

    pub fn add_vbd(&mut self, vbd: VbdDeviceConfig) -> &mut Self {
        self.vbds.push(vbd);
        self
    }

    pub fn get_vbds(&self) -> &Vec<VbdDeviceConfig> {
        &self.vbds
    }

    pub fn add_fs9p(&mut self, fs9p: Fs9pDeviceConfig) -> &mut Self {
        self.fs9ps.push(fs9p);
        self
    }

    pub fn get_fs9ps(&self) -> &Vec<Fs9pDeviceConfig> {
        &self.fs9ps
    }

    pub fn pci(&mut self, pci: PciRootDeviceConfig) -> &mut Self {
        self.pci = Some(pci);
        self
    }

    pub fn get_pci(&self) -> &Option<PciRootDeviceConfig> {
        &self.pci
    }

    pub fn add_extra_key(&mut self, key: impl AsRef<str>, value: impl ToString) -> &mut Self {
        self.extra_keys
            .insert(key.as_ref().to_string(), value.to_string());
        self
    }

    pub fn get_extra_keys(&self) -> &HashMap<String, String> {
        &self.extra_keys
    }

    pub fn add_rw_path(&mut self, path: impl AsRef<str>) -> &mut Self {
        self.extra_rw_paths.push(path.as_ref().to_string());
        self
    }

    pub fn get_rw_paths(&self) -> &Vec<String> {
        &self.extra_rw_paths
    }

    pub fn start(&mut self, start: bool) -> &mut Self {
        self.start = start;
        self
    }

    pub fn get_start(&self) -> bool {
        self.start
    }

    pub fn done(self) -> Self {
        self
    }

    pub(crate) async fn prepare(
        &mut self,
        domid: u32,
        call: &XenCall,
        platform: &PlatformDomainInfo,
    ) -> Result<()> {
        if let Some(pci) = self.pci.as_mut() {
            pci.prepare(domid, call).await?;
        }

        for channel in &mut self.channels {
            channel.prepare(platform).await?;
        }

        Ok(())
    }
}

pub struct DomainResult {
    pub platform: PlatformDomainInfo,
    pub channels: Vec<DeviceResult>,
    pub vifs: Vec<DeviceResult>,
    pub vbds: Vec<BlockDeviceResult>,
    pub fs9ps: Vec<DeviceResult>,
    pub pci: Option<DeviceResult>,
}
