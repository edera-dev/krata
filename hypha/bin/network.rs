use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use hypha::network::HyphaNetwork;

#[derive(Parser, Debug)]
struct NetworkArgs {
    #[arg(short, long)]
    interface: String,
    #[arg(short, long, default_value = "192.168.42.1/24")]
    network: String,
}

fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    let args = NetworkArgs::parse();
    let mut network = HyphaNetwork::new(&args.interface, &[&args.network])?;
    network.run()?;
    Ok(())
}
