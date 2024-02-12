use anyhow::Result;
use clap::Parser;
use env_logger::Env;
use hyphanet::NetworkService;

#[derive(Parser, Debug)]
struct NetworkArgs {}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    let _ = NetworkArgs::parse();
    let mut service = NetworkService::new()?;
    service.watch().await
}
