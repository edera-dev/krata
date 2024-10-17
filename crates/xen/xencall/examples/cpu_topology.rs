use xencall::error::Result;
use xencall::XenCall;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let call = XenCall::open(0)?;
    let physinfo = call.phys_info().await?;
    println!("{:?}", physinfo);
    let topology = call.cpu_topology().await?;
    println!("{:?}", topology);
    Ok(())
}
