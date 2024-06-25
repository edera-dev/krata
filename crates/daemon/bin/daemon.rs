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
        .filter_level(LevelFilter::Trace)
        .parse_default_env()
        .filter(Some("backhand::filesystem::writer"), LevelFilter::Warn);

    if let Ok(f_addr) = std::env::var("KRATA_FLUENT_ADDR") {
        println!("KRATA_FLUENT_ADDR set to {f_addr}");
        let target = SocketAddr::from_str(f_addr.as_str())?;
        builder.target(Target::Pipe(Box::new(TcpStream::connect(target)?)));
    }

    let ev = std::env::vars()
        .into_iter()
        .fold(String::new(), |mut acc, (k, v)| {
            acc.push_str(&format!("{k}={v}\n"));
            acc
        });

    std::fs::write("/var/log/krata/ev", ev)?;

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
