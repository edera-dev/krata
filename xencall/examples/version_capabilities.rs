use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    let call = XenCall::open()?;
    let info = call.get_version_capabilities()?;
    println!("{:?}", info);
    Ok(())
}
