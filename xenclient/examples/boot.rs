use std::fs::read;
use std::{env, process};
use xencall::domctl::DomainControl;
use xencall::memory::MemoryControl;
use xencall::sys::CreateDomain;
use xencall::XenCall;
use xenclient::boot::BootSetup;
use xenclient::elfloader::ElfImageLoader;
use xenclient::XenClientError;

fn main() -> Result<(), XenClientError> {
    env_logger::init();

    let args: Vec<String> = env::args().collect();
    if args.len() != 3 {
        println!("usage: boot <kernel-image> <initrd>");
        process::exit(1);
    }
    let kernel_image_path = args.get(1).expect("argument not specified");
    let initrd_path = args.get(2).expect("argument not specified");
    let call = XenCall::open()?;
    let domctl = DomainControl::new(&call);
    let domain = CreateDomain {
        max_vcpus: 1,
        ..Default::default()
    };
    let domid = domctl.create_domain(domain)?;
    let result = boot(
        domid,
        kernel_image_path.as_str(),
        initrd_path.as_str(),
        &call,
        &domctl,
    );
    domctl.destroy_domain(domid)?;
    result?;
    println!("domain destroyed: {}", domid);
    Ok(())
}

fn boot(
    domid: u32,
    kernel_image_path: &str,
    initrd_path: &str,
    call: &XenCall,
    domctl: &DomainControl,
) -> Result<(), XenClientError> {
    println!("domain created: {:?}", domid);
    let image_loader = ElfImageLoader::load_file_kernel(kernel_image_path)?;
    let memctl = MemoryControl::new(call);
    let mut boot = BootSetup::new(call, domctl, &memctl, domid);
    let initrd = read(initrd_path)?;
    let mut state = boot.initialize(&image_loader, initrd.as_slice(), 512)?;
    boot.boot(&mut state, "debug")?;
    Ok(())
}
