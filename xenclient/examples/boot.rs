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
    println!("domain created: {:?}", domid);
    let image_loader = ElfImageLoader::load_file_kernel(kernel_image_path.as_str())?;
    let memctl = MemoryControl::new(&call);
    let mut boot = BootSetup::new(&call, &domctl, &memctl, domid);
    let mut state = boot.initialize(&image_loader, 512 * 1024)?;
    boot.boot(&mut state, "debug")?;
    domctl.destroy_domain(domid)?;
    println!("domain destroyed: {}", domid);
    Ok(())
}
