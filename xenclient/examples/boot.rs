use std::alloc::Layout;
use std::{env, process};
use xencall::domctl::DomainControl;
use xencall::sys::CreateDomain;
use xencall::XenCall;
use xenclient::boot::BootImageLoader;
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
    let boot = ElfImageLoader::load_file_kernel(kernel_image_path.as_str())?;
    let ptr = unsafe { std::alloc::alloc(Layout::from_size_align(128 * 1024 * 1024, 16).unwrap()) };
    let info = boot.load(ptr)?;
    println!("loaded kernel image into memory: {:?}", info);
    // The address calculations don't make sense here and I am certain something
    // is wrong up the stack.
    // if info.virt_hypercall != XEN_UNSET_ADDR {
    //     domctl.hypercall_init(domid, info.virt_hypercall)?;
    // }
    domctl.destroy_domain(domid)?;
    println!("domain destroyed: {}", domid);
    Ok(())
}
