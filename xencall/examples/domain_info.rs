use xencall::domctl::DomainControl;
use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    let mut call = XenCall::open()?;
    let mut domctl: DomainControl = DomainControl::new(&mut call);
    let info = domctl.get_domain_info(1)?;
    println!("{:?}", info);
    Ok(())
}
