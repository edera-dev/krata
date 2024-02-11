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
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    let args = NetworkArgs::parse();
    let mut service = NetworkService::new(args.ipv4_network, args.ipv6_network)?;
    service.watch().await?;
    Ok(())
}
