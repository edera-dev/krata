use std::sync::Arc;

use crate::{
    boot::{BootSetup, BootSetupPlatform},
    elfloader::ElfImageLoader,
};
use uuid::Uuid;
use xencall::XenCall;

use crate::error::Result;

pub struct BaseDomainManager<P: BootSetupPlatform> {
    call: XenCall,
    pub platform: Arc<P>,
}

impl<P: BootSetupPlatform> BaseDomainManager<P> {
    pub async fn new(call: XenCall, platform: P) -> Result<BaseDomainManager<P>> {
        Ok(BaseDomainManager {
            call,
            platform: Arc::new(platform),
        })
    }

    pub async fn create(&self, config: BaseDomainConfig) -> Result<CreatedDomain> {
        let mut domain = self.platform.create_domain(config.enable_iommu);
        domain.handle = config.uuid.into_bytes();
        domain.max_vcpus = config.max_vcpus;
        let domid = self.call.create_domain(domain).await?;
        self.call.set_max_vcpus(domid, config.max_vcpus).await?;
        self.call
            .set_max_mem(domid, (config.max_mem_mb * 1024) + 2048)
            .await?;
        let loader = ElfImageLoader::load_file_kernel(&config.kernel)?;
        let platform = (*self.platform).clone();
        let mut boot = BootSetup::new(self.call.clone(), domid, platform, loader, None);
        let mut domain = boot
            .initialize(
                &config.initrd,
                config.target_mem_mb,
                config.max_mem_mb,
                config.max_vcpus,
                &config.cmdline,
            )
            .await?;
        boot.boot(&mut domain).await?;
        Ok(CreatedDomain {
            domid,
            store_evtchn: domain.store_evtchn,
            store_mfn: domain.store_mfn,
            console_evtchn: domain.console_evtchn,
            console_mfn: domain.console_mfn,
        })
    }

    pub async fn destroy(&self, domid: u32) -> Result<()> {
        self.call.destroy_domain(domid).await?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct BaseDomainConfig {
    pub uuid: Uuid,
    pub owner_domid: u32,
    pub max_vcpus: u32,
    pub max_mem_mb: u64,
    pub target_mem_mb: u64,
    pub kernel: Vec<u8>,
    pub initrd: Vec<u8>,
    pub cmdline: String,
    pub enable_iommu: bool,
}

#[derive(Clone, Debug)]
pub struct CreatedDomain {
    pub domid: u32,
    pub store_evtchn: u32,
    pub store_mfn: u64,
    pub console_evtchn: u32,
    pub console_mfn: u64,
}
