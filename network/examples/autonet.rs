use std::{thread::sleep, time::Duration};

use anyhow::Result;
use hyphanet::autonet::AutoNetworkCollector;

fn main() -> Result<()> {
    let mut collector = AutoNetworkCollector::new()?;
    loop {
        let changeset = collector.read_changes()?;
        println!("{:?}", changeset);
        sleep(Duration::from_secs(2));
    }
}
