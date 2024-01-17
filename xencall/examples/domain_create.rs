use xencall::sys::CreateDomain;
use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    env_logger::init();

    let call = XenCall::open()?;
    let domid = call.create_domain(CreateDomain::default())?;
    println!("created domain {}", domid);
    Ok(())
}
