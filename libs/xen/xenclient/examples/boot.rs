use std::{env, process};
use xenclient::error::Result;
use xenclient::{DomainConfig, XenClient};

fn main() -> Result<()> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        println!("usage: boot <kernel-image> <initrd>");
        process::exit(1);
    }
    let kernel_image_path = args.get(1).expect("argument not specified");
    let initrd_path = args.get(2).expect("argument not specified");
    let mut client = XenClient::open()?;
    let config = DomainConfig {
        backend_domid: 0,
        name: "xenclient-test",
        max_vcpus: 1,
        mem_mb: 512,
        kernel_path: kernel_image_path.as_str(),
        initrd_path: initrd_path.as_str(),
        cmdline: "debug elevator=noop",
        disks: vec![],
        vifs: vec![],
        filesystems: vec![],
        extra_keys: vec![],
    };
    let domid = client.create(&config)?;
    println!("created domain {}", domid);
    Ok(())
}
