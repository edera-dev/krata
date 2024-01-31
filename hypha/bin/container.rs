use std::env;
use anyhow::Result;
use hypha::container::init::ContainerInit;

fn main() -> Result<()> {
    env::set_var("RUST_BACKTRACE", "1");
    env_logger::init();
    let mut container = ContainerInit::new();
    container.init()?;
    Ok(())
}
