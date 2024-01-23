use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    env_logger::init();

    let call = XenCall::open()?;
    let info = call.get_domain_info(1)?;
    println!("{:?}", info);
    Ok(())
}
