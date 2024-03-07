use anyhow::{anyhow, Result};
use env_logger::Env;
use krataguest::init::GuestInit;
use std::env;

#[tokio::main]
async fn main() -> Result<()> {
    env::set_var("RUST_BACKTRACE", "1");
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    if env::var("KRATA_UNSAFE_ALWAYS_ALLOW_INIT").unwrap_or("0".to_string()) != "1" {
        let pid = std::process::id();
        if pid > 3 {
            return Err(anyhow!(
                "not running because the pid of {} indicates this is probably not \
                    the right context for the init daemon. \
                        run with KRATA_UNSAFE_ALWAYS_ALLOW_INIT=1 to bypass this check",
                pid
            ));
        }
    }
    let mut guest = GuestInit::new();
    guest.init().await?;
    Ok(())
}
