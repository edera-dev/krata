use std::{
    net::{SocketAddr, TcpStream},
    str::FromStr,
    sync::{atomic::AtomicBool, Arc},
};

use anyhow::Result;
use clap::Parser;
use env_logger::fmt::Target;
use log::LevelFilter;

use kratad::command::DaemonCommand;

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    let mut builder = env_logger::Builder::new();
    builder
        .filter_level(LevelFilter::Info)
        .parse_default_env()
        .filter(Some("backhand::filesystem::writer"), LevelFilter::Warn);

    if let Ok(f_addr) = std::env::var("KRATA_FLUENT_ADDR") {
        let target = SocketAddr::from_str(f_addr.as_str())?;
        builder.target(Target::Pipe(Box::new(TcpStream::connect(target)?)));
    }

    builder.init();

    mask_sighup()?;

    let command = DaemonCommand::parse();
    command.run().await
}

fn mask_sighup() -> Result<()> {
    let flag = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, flag)?;
    Ok(())
}
