use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use krata::dial::ControlDialAddress;
use kratad::{runtime::Runtime, Daemon};
use std::{
    str::FromStr,
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_value = "unix:///var/lib/krata/daemon.socket")]
    listen: String,
    #[arg(short, long, default_value = "/var/lib/krata")]
    store: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    mask_sighup()?;

    let args = Args::parse();
    let addr = ControlDialAddress::from_str(&args.listen)?;
    let runtime = Runtime::new(args.store.clone()).await?;
    let mut daemon = Daemon::new(args.store.clone(), runtime).await?;
    daemon.listen(addr).await?;
    Ok(())
}

fn mask_sighup() -> Result<()> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, flag)?;
    Ok(())
}
