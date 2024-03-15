use anyhow::Result;
use clap::Parser;
use env_logger::Env;

use kratactl::cli::ControlCommand;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    ControlCommand::parse().run().await
}
