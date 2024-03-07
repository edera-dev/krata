use xencall::error::Result;
use xencall::XenCall;

fn main() -> Result<()> {
    env_logger::init();

    let call = XenCall::open()?;
    let info = call.get_domain_info(1)?;
    println!("{:?}", info);
    Ok(())
}
