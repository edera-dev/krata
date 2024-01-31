use xencall::error::Result;
use xencall::XenCall;

fn main() -> Result<()> {
    env_logger::init();

    let call = XenCall::open()?;
    let info = call.get_version_capabilities()?;
    println!("{:?}", info);
    Ok(())
}
