use std::{env, process};
use tokio::fs;
use uuid::Uuid;
use xenclient::error::Result;
use xenclient::{DomainConfig, XenClient};
use xenplatform::domain::BaseDomainConfig;

#[cfg(target_arch = "x86_64")]
type RuntimePlatform = xenplatform::x86pv::X86PvPlatform;

#[cfg(not(target_arch = "x86_64"))]
type RuntimePlatform = xenplatform::unsupported::UnsupportedPlatform;

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
    let client = XenClient::new(0, RuntimePlatform::new()).await?;
    let config = DomainConfig {
        base: BaseDomainConfig {
            uuid: Uuid::new_v4(),
            max_vcpus: 1,
            max_mem_mb: 512,
            target_mem_mb: 512,
            enable_iommu: true,
            kernel: fs::read(&kernel_image_path).await?,
            initrd: fs::read(&initrd_path).await?,
            cmdline: "earlyprintk=xen earlycon=xen console=hvc0 init=/init".to_string(),
            owner_domid: 0,
        },
        backend_domid: 0,
        name: "xenclient-test".to_string(),
        swap_console_backend: None,
        disks: vec![],
        channels: vec![],
        vifs: vec![],
        pcis: vec![],
        filesystems: vec![],
        extra_keys: vec![],
        extra_rw_paths: vec![],
    };
    let created = client.create(&config).await?;
    println!("created domain {}", created.domid);
    Ok(())
}
