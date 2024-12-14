use std::sync::Arc;
use std::{env, process};
use tokio::fs;
use uuid::Uuid;
use xenclient::error::Result;
use xenclient::tx::channel::ChannelDeviceConfig;
use xenclient::{config::DomainConfig, XenClient};
use xenplatform::domain::{
    KernelFormat, PlatformDomainConfig, PlatformKernelConfig, PlatformOptions,
    PlatformResourcesConfig,
};
use xenplatform::RuntimePlatformType;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        println!("usage: boot <kernel-image> <initrd>");
        process::exit(1);
    }
    let kernel_image_path = args.get(1).expect("argument not specified");
    let initrd_path = args.get(2).expect("argument not specified");
    let client = XenClient::new().await?;

    #[cfg(target_arch = "x86_64")]
    let runtime_platform = RuntimePlatformType::Pv;
    #[cfg(not(target_arch = "x86_64"))]
    let runtime_platform = RuntimePlatformType::Unsupported;

    let mut config = DomainConfig::new();
    config.platform(PlatformDomainConfig {
        uuid: Uuid::new_v4(),
        platform: runtime_platform,
        kernel: PlatformKernelConfig {
            data: Arc::new(fs::read(&kernel_image_path).await?),
            format: KernelFormat::ElfCompressed,
            cmdline: "earlyprintk=xen earlycon=xen console=hvc0 init=/init".to_string(),
            initrd: Some(Arc::new(fs::read(&initrd_path).await?)),
        },
        resources: PlatformResourcesConfig {
            max_vcpus: 1,
            assigned_vcpus: 1,
            max_memory_mb: 512,
            assigned_memory_mb: 512,
        },
        options: PlatformOptions { iommu: true },
    });
    config.name("xenclient-test");
    let mut channel = ChannelDeviceConfig::new();
    channel.default_console().backend_initialized();
    config.add_channel(channel);
    let created = client.create(config).await?;
    println!("created domain {}", created.platform.domid);
    Ok(())
}
