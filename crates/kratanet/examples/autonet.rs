use std::time::Duration;

use anyhow::Result;
use kratanet::autonet::AutoNetworkCollector;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<()> {
    let mut collector = AutoNetworkCollector::new().await?;
    loop {
        let changeset = collector.read_changes().await?;
        println!("{:?}", changeset);
        sleep(Duration::from_secs(2)).await;
    }
}
