use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    env_logger::init();

    let call = XenCall::open()?;
    let info = call.get_version_capabilities()?;
    println!("{:?}", info);
    Ok(())
}
