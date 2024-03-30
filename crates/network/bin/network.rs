use std::str::FromStr;

use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use krata::dial::ControlDialAddress;
use kratanet::NetworkService;

#[derive(Parser, Debug)]
struct NetworkArgs {
    #[arg(short, long, default_value = "unix:///var/lib/krata/daemon.socket")]
    connection: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();
    let args = NetworkArgs::parse();
    let control_dial_address = ControlDialAddress::from_str(&args.connection)?;
    let mut service = NetworkService::new(control_dial_address).await?;
    service.watch().await
}
