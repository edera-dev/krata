use std::sync::Arc;
use std::{env, process};
use tokio::fs;
use uuid::Uuid;
use xenclient::config::{DomainConfig, DomainResult};
use xenclient::error::Result;
use xenclient::tx::channel::ChannelDeviceConfig;
use xenclient::XenClient;
use xenplatform::domain::{
    KernelFormat, PlatformDomainConfig, PlatformKernelConfig, PlatformOptions,
    PlatformResourcesConfig,
};
use xenplatform::elfloader::ElfImageLoader;
use xenplatform::RuntimePlatformType;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        println!("usage: boot-speed <kernel-image>");
        process::exit(1);
    }
    let kernel_path = args.get(1).expect("argument not specified");
    let kernel = Arc::new(fs::read(kernel_path).await?);
    let kernel = ElfImageLoader::load(kernel)?.into_elf_bytes();
    let client = XenClient::new().await?;

    for i in 0..5u32 {
        let start = std::time::Instant::now();
        let domain = create_domain(&client, kernel.clone(), i).await?;
        let end = std::time::Instant::now();
        let duration = end - start;
        println!("boot setup time: {:?}", duration);
        client.destroy(domain.platform.domid).await?;
    }
    Ok(())
}

async fn create_domain(client: &XenClient, kernel: Arc<Vec<u8>>, i: u32) -> Result<DomainResult> {
    let mut config = DomainConfig::new();
    config.platform(PlatformDomainConfig {
        uuid: Uuid::new_v4(),
        platform: RuntimePlatformType::supported(),
        kernel: PlatformKernelConfig {
            data: kernel,
            format: KernelFormat::ElfUncompressed,
            cmdline: "earlyprintk=xen earlycon=xen console=hvc0 init=/init".to_string(),
            initrd: None,
        },
        resources: PlatformResourcesConfig {
            max_vcpus: 1,
            assigned_vcpus: 1,
            max_memory_mb: 512,
            assigned_memory_mb: 512,
        },
        options: PlatformOptions { iommu: true },
    });
    config.name(format!("xenboot-{}", i));
    config.start(false);
    let mut channel = ChannelDeviceConfig::new();
    channel.default_console().backend_initialized();
    config.add_channel(channel);
    client.create(config).await
}
