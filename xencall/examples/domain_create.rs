use xencall::domctl::DomainControl;
use xencall::sys::CreateDomain;
use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    let call = XenCall::open()?;
    let domctl: DomainControl = DomainControl::new(&call);
    let info = domctl.create_domain(CreateDomain::default())?;
    println!("created domain {}", info.domid);
    Ok(())
}
