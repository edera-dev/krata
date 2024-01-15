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
    if args.len() != 2 {
        println!("usage: boot <kernel-image>");
        process::exit(1);
    }
    let kernel_image_path = args.get(1).expect("argument not specified");
    let call = XenCall::open()?;
    let domctl = DomainControl::new(&call);
    let domid = domctl.create_domain(CreateDomain::default())?;
    domctl.pause_domain(domid)?;
    domctl.set_max_vcpus(domid, 1)?;
    let result = boot(domid, kernel_image_path.as_str(), &call, &domctl);
    domctl.destroy_domain(domid)?;
    result?;
    println!("domain destroyed: {}", domid);
    Ok(())
}

fn boot(
    domid: u32,
    kernel_image_path: &str,
    call: &XenCall,
    domctl: &DomainControl,
) -> Result<(), XenClientError> {
    println!("domain created: {:?}", domid);
    let image_loader = ElfImageLoader::load_file_kernel(kernel_image_path)?;
    let memctl = MemoryControl::new(call);
    let mut boot = BootSetup::new(call, domctl, &memctl, domid);
    let mut state = boot.initialize(&image_loader, 128 * 1024)?;
    boot.boot(&mut state, "debug")?;
    Ok(())
}
