use xencall::error::Result;
use xencall::sys::CreateDomain;
use xencall::XenCall;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let call = XenCall::open(0)?;
    let domid = call.create_domain(CreateDomain::default()).await?;
    println!("created domain {}", domid);
    Ok(())
}
