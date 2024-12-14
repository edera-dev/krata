use std::sync::Arc;

use crate::{
    boot::BootDomain, elfloader::ElfImageLoader, error::Error, ImageLoader, RuntimePlatform,
    RuntimePlatformType,
};
use log::warn;
use uuid::Uuid;
use xencall::XenCall;

use crate::error::Result;

pub const XEN_EXTRA_MEMORY_KB: u64 = 2048;

pub struct PlatformDomainManager {
    call: XenCall,
}

impl PlatformDomainManager {
    pub async fn new(call: XenCall) -> Result<PlatformDomainManager> {
        Ok(PlatformDomainManager { call })
    }

    fn max_memory_kb(resources: &PlatformResourcesConfig) -> u64 {
        (resources.max_memory_mb * 1024) + XEN_EXTRA_MEMORY_KB
    }

    async fn create_base_domain(
        &self,
        config: &PlatformDomainConfig,
        platform: &RuntimePlatform,
    ) -> Result<u32> {
        let mut domain = platform.create_domain(config.options.iommu);
        domain.handle = config.uuid.into_bytes();
        domain.max_vcpus = config.resources.max_vcpus;
        let domid = self.call.create_domain(domain).await?;
        Ok(domid)
    }

    async fn configure_domain_resources(
        &self,
        domid: u32,
        config: &PlatformDomainConfig,
    ) -> Result<()> {
        self.call
            .set_max_vcpus(domid, config.resources.max_vcpus)
            .await?;
        self.call
            .set_max_mem(
                domid,
                PlatformDomainManager::max_memory_kb(&config.resources),
            )
            .await?;
        Ok(())
    }

    async fn create_internal(
        &self,
        domid: u32,
        config: &PlatformDomainConfig,
        mut platform: RuntimePlatform,
    ) -> Result<BootDomain> {
        self.configure_domain_resources(domid, config).await?;
        let kernel = config.kernel.clone();
        let loader = tokio::task::spawn_blocking(move || match kernel.format {
            KernelFormat::ElfCompressed => ElfImageLoader::load(kernel.data),
            KernelFormat::ElfUncompressed => Ok(ElfImageLoader::new(kernel.data)),
        })
        .await
        .map_err(Error::AsyncJoinError)??;
        let loader = ImageLoader::Elf(loader);
        let mut domain = platform
            .initialize(
                domid,
                self.call.clone(),
                &loader,
                &config.kernel,
                &config.resources,
            )
            .await?;
        platform.boot(domid, self.call.clone(), &mut domain).await?;
        Ok(domain)
    }

    pub async fn create(&self, config: PlatformDomainConfig) -> Result<PlatformDomainInfo> {
        let platform = config.platform.create();
        let domid = self.create_base_domain(&config, &platform).await?;
        let domain = match self.create_internal(domid, &config, platform).await {
            Ok(domain) => domain,
            Err(error) => {
                if let Err(destroy_fail) = self.call.destroy_domain(domid).await {
                    warn!(
                        "failed to destroy failed domain {}: {}",
                        domid, destroy_fail
                    );
                }
                return Err(error);
            }
        };
        Ok(PlatformDomainInfo {
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
pub struct PlatformDomainConfig {
    pub uuid: Uuid,
    pub platform: RuntimePlatformType,
    pub resources: PlatformResourcesConfig,
    pub kernel: PlatformKernelConfig,
    pub options: PlatformOptions,
}

#[derive(Clone, Debug)]
pub struct PlatformKernelConfig {
    pub data: Arc<Vec<u8>>,
    pub format: KernelFormat,
    pub initrd: Option<Arc<Vec<u8>>>,
    pub cmdline: String,
}

#[derive(Clone, Debug)]
pub struct PlatformResourcesConfig {
    pub max_vcpus: u32,
    pub assigned_vcpus: u32,
    pub max_memory_mb: u64,
    pub assigned_memory_mb: u64,
}

#[derive(Clone, Debug)]
pub struct PlatformOptions {
    pub iommu: bool,
}

#[derive(Clone, Debug)]
pub enum KernelFormat {
    ElfUncompressed,
    ElfCompressed,
}

#[derive(Clone, Debug)]
pub struct PlatformDomainInfo {
    pub domid: u32,
    pub store_evtchn: u32,
    pub store_mfn: u64,
    pub console_evtchn: u32,
    pub console_mfn: u64,
}
