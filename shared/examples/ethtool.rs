use std::env;

use anyhow::Result;
use krata::ethtool::EthtoolHandle;

fn main() -> Result<()> {
    let args = env::args().collect::<Vec<String>>();
    let interface = args.get(1).unwrap();
    let mut handle = EthtoolHandle::new()?;
    handle.set_gso(interface, false)?;
    handle.set_tso(interface, false)?;
    Ok(())
}
