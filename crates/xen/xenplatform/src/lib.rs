pub mod boot;
pub mod elfloader;
pub mod error;
pub mod mem;
pub mod sys;

use boot::{BootDomain, BootImageInfo, BootImageLoader, BootSetupPlatform};
use domain::{PlatformKernelConfig, PlatformResourcesConfig};
use elfloader::ElfImageLoader;
use error::Result;
use unsupported::UnsupportedPlatform;
use xencall::{sys::CreateDomain, XenCall};

use crate::error::Error;

pub mod domain;
pub mod unsupported;
#[cfg(target_arch = "x86_64")]
pub mod x86pv;

#[derive(Clone)]
pub enum ImageLoader {
    Elf(ElfImageLoader),
}

impl ImageLoader {
    async fn parse(&self, hvm: bool) -> Result<BootImageInfo> {
        match self {
            ImageLoader::Elf(elf) => elf.parse(hvm).await,
        }
    }

    async fn load(&self, image_info: &BootImageInfo, dst: &mut [u8]) -> Result<()> {
        match self {
            ImageLoader::Elf(elf) => elf.load(image_info, dst).await,
        }
    }
}

#[derive(Clone, Debug, PartialEq, PartialOrd, Eq, Ord)]
pub enum RuntimePlatformType {
    Unsupported,
    #[cfg(target_arch = "x86_64")]
    Pv,
}

impl RuntimePlatformType {
    pub fn create(&self) -> RuntimePlatform {
        match self {
            RuntimePlatformType::Unsupported => {
                RuntimePlatform::Unsupported(UnsupportedPlatform::new())
            }
            #[cfg(target_arch = "x86_64")]
            RuntimePlatformType::Pv => RuntimePlatform::Pv(x86pv::X86PvPlatform::new()),
        }
    }

    pub fn supported() -> RuntimePlatformType {
        if cfg!(target_arch = "x86_64") {
            RuntimePlatformType::Pv
        } else {
            RuntimePlatformType::Unsupported
        }
    }
}

#[allow(clippy::large_enum_variant)]
pub enum RuntimePlatform {
    Unsupported(UnsupportedPlatform),
    #[cfg(target_arch = "x86_64")]
    Pv(x86pv::X86PvPlatform),
}

impl RuntimePlatform {
    #[allow(clippy::too_many_arguments)]
    pub async fn initialize(
        &mut self,
        domid: u32,
        call: XenCall,
        image_loader: &ImageLoader,
        kernel: &PlatformKernelConfig,
        resources: &PlatformResourcesConfig,
    ) -> Result<BootDomain> {
        match self {
            RuntimePlatform::Unsupported(unsupported) => {
                unsupported
                    .initialize(domid, call, image_loader, kernel, resources)
                    .await
            }
            #[cfg(target_arch = "x86_64")]
            RuntimePlatform::Pv(pv) => {
                pv.initialize(domid, call, image_loader, kernel, resources)
                    .await
            }
        }
    }

    pub async fn boot(&mut self, domid: u32, call: XenCall, domain: &mut BootDomain) -> Result<()> {
        match self {
            RuntimePlatform::Unsupported(unsupported) => {
                unsupported.boot(domid, call, domain).await
            }
            #[cfg(target_arch = "x86_64")]
            RuntimePlatform::Pv(pv) => pv.boot(domid, call, domain).await,
        }
    }

    pub fn create_domain(&self, enable_iommu: bool) -> CreateDomain {
        match self {
            RuntimePlatform::Unsupported(unsupported) => unsupported.create_domain(enable_iommu),
            #[cfg(target_arch = "x86_64")]
            RuntimePlatform::Pv(pv) => pv.create_domain(enable_iommu),
        }
    }
}
