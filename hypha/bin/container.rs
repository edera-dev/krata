use hypha::container::init::ContainerInit;
use hypha::error::Result;

fn main() -> Result<()> {
    env_logger::init();
    let mut container = ContainerInit::new();
    container.init()?;
    Ok(())
}
