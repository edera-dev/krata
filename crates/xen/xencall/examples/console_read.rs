use xencall::error::Result;
use xencall::XenCall;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let call = XenCall::open(0)?;
    let index = 0_u32;
    let (buf, newindex) = call.read_console_ring_raw(false, index).await?;

    match std::str::from_utf8(&buf[..newindex as usize]) {
        Ok(v) => print!("{}", v),
        _ => panic!("unable to decode Xen console messages"),
    };

    Ok(())
}
