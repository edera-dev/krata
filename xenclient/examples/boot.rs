use std::alloc::Layout;
use xencall::domctl::DomainControl;
use xencall::sys::CreateDomain;
use xencall::XenCall;
use xenclient::boot::BootImageLoader;
use xenclient::elfloader::ElfImageLoader;
use xenclient::XenClientError;

fn main() -> Result<(), XenClientError> {
    let call = XenCall::open()?;
    let domctl = DomainControl::new(&call);
    let _domain = domctl.create_domain(CreateDomain::default())?;
    let boot = ElfImageLoader::load_file_kernel("/boot/vmlinuz-6.1.0-17-amd64")?;
    let ptr = unsafe { std::alloc::alloc(Layout::from_size_align(128 * 1024 * 1024, 16).unwrap()) };
    let info = boot.load(ptr)?;
    println!("{:?}", info);
    Ok(())
}
