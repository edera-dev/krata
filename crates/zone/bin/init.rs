use anyhow::Result;
use env_logger::Env;
use kratazone::{death, init::ZoneInit};
use log::error;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    env::set_var("RUST_BACKTRACE", "1");
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    let mut zone = ZoneInit::new();
    if let Err(error) = zone.init().await {
        error!("failed to initialize zone: {}", error);
        death(127).await?;
        return Ok(());
    }
    death(1).await?;
    Ok(())
}
