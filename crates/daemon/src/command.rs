use anyhow::Result;
use clap::{CommandFactory, Parser};
use krata::dial::ControlDialAddress;
use std::str::FromStr;

use crate::Daemon;

#[derive(Parser)]
#[command(version, about = "krata isolation engine daemon")]
pub struct DaemonCommand {
    #[arg(
        short,
        long,
        default_value = "unix:///var/lib/krata/daemon.socket",
        help = "Listen address"
    )]
    listen: String,
    #[arg(short, long, default_value = "/var/lib/krata", help = "Storage path")]
    store: String,
}

impl DaemonCommand {
    pub async fn run(self) -> Result<()> {
        let addr = ControlDialAddress::from_str(&self.listen)?;
        let mut daemon = Daemon::new(self.store.clone()).await?;
        daemon.listen(addr).await?;
        Ok(())
    }

    pub fn version() -> String {
        DaemonCommand::command()
            .get_version()
            .unwrap_or("unknown")
            .to_string()
    }
}
