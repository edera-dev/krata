use std::{env, process};
use xenclient::create::DomainConfig;
use xenclient::{XenClient, XenClientError};

fn main() -> Result<(), XenClientError> {
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
        max_vcpus: 1,
        mem_mb: 512,
        kernel_path: kernel_image_path.to_string(),
        initrd_path: initrd_path.to_string(),
        cmdline: "debug elevator=noop".to_string(),
    };
    let domid = client.create(config)?;
    println!("created domain {}", domid);
    Ok(())
}
