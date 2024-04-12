use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use krata::dial::ControlDialAddress;
use kratad::Daemon;
use log::LevelFilter;
use std::{
    str::FromStr,
    sync::{atomic::AtomicBool, Arc},
};

#[derive(Parser)]
struct DaemonCommand {
    #[arg(short, long, default_value = "unix:///var/lib/krata/daemon.socket")]
    listen: String,
    #[arg(short, long, default_value = "/var/lib/krata")]
    store: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .filter(Some("backhand::filesystem::writer"), LevelFilter::Warn)
        .init();
    mask_sighup()?;

    let args = DaemonCommand::parse();
    let addr = ControlDialAddress::from_str(&args.listen)?;

    let mut daemon = Daemon::new(args.store.clone()).await?;
    daemon.listen(addr).await?;
    Ok(())
}

fn mask_sighup() -> Result<()> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, flag)?;
    Ok(())
}
