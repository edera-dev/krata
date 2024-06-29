use xencall::error::Result;
use xencall::sys::CpuId;
use xencall::XenCall;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let call = XenCall::open(0)?;
    let physinfo = call.phys_info().await?;
    println!("{:?}", physinfo);
    let topology = call.cpu_topology().await?;
    println!("{:?}", topology);
    call.set_cpufreq_gov(CpuId::All, "performance").await?;
    call.set_cpufreq_gov(CpuId::Single(0), "performance")
        .await?;
    call.set_turbo_mode(CpuId::All, true).await?;
    Ok(())
}
