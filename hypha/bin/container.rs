use anyhow::Result;
use hypha::container::init::ContainerInit;

fn main() -> Result<()> {
    env_logger::init();
    let mut container = ContainerInit::new();
    container.init()?;
    Ok(())
}
