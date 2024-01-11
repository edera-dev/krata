use std::{env, process};
use xencall::domctl::DomainControl;
use xencall::memory::MemoryControl;
use xencall::sys::CreateDomain;
use xencall::XenCall;
use xenclient::boot::{BootImageLoader, BootSetup};
use xenclient::elfloader::ElfImageLoader;
use xenclient::XenClientError;

fn main() -> Result<(), XenClientError> {
    let args: Vec<String> = env::args().collect();
    if args.len() != 2 {
        println!("usage: boot <kernel-image>");
        process::exit(1);
    }
    let kernel_image_path = args.get(1).expect("argument not specified");
    let call = XenCall::open()?;
    let domctl = DomainControl::new(&call);
    let domid = domctl.create_domain(CreateDomain::default())?;
    let domain = domctl.get_domain_info(domid)?;
    println!("domain created: {:?}", domain);
    let image_loader = ElfImageLoader::load_file_kernel(kernel_image_path.as_str())?;
    let image_info = image_loader.parse()?;
    println!("loaded kernel image into memory: {:?}", image_info);
    let memctl = MemoryControl::new(&call);
    let mut boot = BootSetup::new(&call, &domctl, &memctl, domid);
    boot.initialize(image_info, 512 * 1024)?;
    domctl.destroy_domain(domid)?;
    println!("domain destroyed: {}", domid);
    Ok(())
}
