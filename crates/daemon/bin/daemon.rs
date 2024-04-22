use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use kratad::command::DaemonCommand;
use log::LevelFilter;
use std::sync::{atomic::AtomicBool, Arc};

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .filter(Some("backhand::filesystem::writer"), LevelFilter::Warn)
        .init();
    mask_sighup()?;

    let command = DaemonCommand::parse();
    command.run().await
}

fn mask_sighup() -> Result<()> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, flag)?;
    Ok(())
}
