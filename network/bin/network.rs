use std::str::FromStr;

use advmac::MacAddr6;
use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use hyphanet::NetworkService;

#[derive(Parser, Debug)]
struct NetworkArgs {
    #[arg(long, default_value = "192.168.42.1/24")]
    ipv4_network: String,

    #[arg(long, default_value = "fe80::1/10")]
    ipv6_network: String,

    #[arg(long)]
    force_mac_address: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    let args = NetworkArgs::parse();

    let force_mac_address = if let Some(mac_str) = args.force_mac_address {
        Some(MacAddr6::from_str(&mac_str)?)
    } else {
        None
    };

    let mut service = NetworkService::new(args.ipv4_network, args.ipv6_network, force_mac_address)?;
    service.watch().await?;
    Ok(())
}
