use xencall::domctl::DomainControl;
use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    env_logger::init();

    let call = XenCall::open()?;
    let domctl: DomainControl = DomainControl::new(&call);
    let context = domctl.get_vcpu_context(224, 0)?;
    println!("{:?}", context);
    Ok(())
}
