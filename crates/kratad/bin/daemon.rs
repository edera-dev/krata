use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use krata::dial::ControlDialAddress;
use kratad::Daemon;
use kratart::Runtime;
use log::error;
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
    #[arg(long, default_value = "false")]
    no_load_guest_tab: bool,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    mask_sighup()?;

    let args = Args::parse();
    let addr = ControlDialAddress::from_str(&args.listen)?;
    let runtime = Runtime::new(args.store.clone()).await?;
    let mut daemon = Daemon::new(args.store.clone(), runtime).await?;
    if !args.no_load_guest_tab {
        if let Err(error) = daemon.load_guest_tab().await {
            error!("failed to load guest tab: {}", error);
        }
    }
    daemon.listen(addr).await?;
    Ok(())
}

fn mask_sighup() -> Result<()> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, flag)?;
    Ok(())
}
