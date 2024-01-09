use xencall::domctl::DomainControl;
use xencall::sys::CreateDomain;
use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    let mut call = XenCall::open()?;
    let mut domctl: DomainControl = DomainControl::new(&mut call);
    let info = domctl.create_domain(CreateDomain::default())?;
    println!("created domain {}", info.domid);
    Ok(())
}
