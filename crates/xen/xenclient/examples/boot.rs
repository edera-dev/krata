use std::{env, process};
use tokio::fs;
use xenclient::error::Result;
use xenclient::x86pvh::X86PvhPlatform;
use xenclient::{DomainConfig, XenClient};

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
    let client = XenClient::new(0, X86PvhPlatform::new()).await?;
    let config = DomainConfig {
        backend_domid: 0,
        name: "xenclient-test".to_string(),
        max_vcpus: 1,
        mem_mb: 512,
        kernel: fs::read(&kernel_image_path).await?,
        initrd: fs::read(&initrd_path).await?,
        cmdline: "earlyprintk=xen earlycon=xen console=hvc0 init=/init".to_string(),
        swap_console_backend: None,
        disks: vec![],
        channels: vec![],
        vifs: vec![],
        pcis: vec![],
        filesystems: vec![],
        extra_keys: vec![],
        extra_rw_paths: vec![],
        event_channels: vec![],
    };
    let created = client.create(&config).await?;
    println!("created domain {}", created.domid);
    Ok(())
}
