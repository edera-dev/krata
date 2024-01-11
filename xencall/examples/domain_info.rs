use xencall::domctl::DomainControl;
use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    env_logger::init();

    let call = XenCall::open()?;
    let domctl: DomainControl = DomainControl::new(&call);
    let info = domctl.get_domain_info(1)?;
    println!("{:?}", info);
    Ok(())
}
