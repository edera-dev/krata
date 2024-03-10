use xencall::error::Result;
use xencall::XenCall;

fn main() -> Result<()> {
    env_logger::init();

    let call = XenCall::open(0)?;
    let context = call.get_vcpu_context(224, 0)?;
    println!("{:?}", context);
    Ok(())
}
