use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    env_logger::init();

    let call = XenCall::open()?;
    let context = call.get_vcpu_context(224, 0)?;
    println!("{:?}", context);
    Ok(())
}
