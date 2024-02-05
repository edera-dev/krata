use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use hypha::network::NetworkService;

#[derive(Parser, Debug)]
struct NetworkArgs {
    #[arg(short, long, default_value = "192.168.42.1/24")]
    network: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    let args = NetworkArgs::parse();
    let mut service = NetworkService::new(args.network)?;
    service.watch().await?;
    Ok(())
}
