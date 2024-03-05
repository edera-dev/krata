use anyhow::{anyhow, Result};
use clap::Parser;
use env_logger::Env;
use kratad::{runtime::Runtime, Daemon};
use tokio_listener::ListenerAddressLFlag;

#[derive(Parser)]
struct Args {
    #[clap(flatten)]
    listener: ListenerAddressLFlag,
    #[arg(short, long, default_value = "/var/lib/krata")]
    store: String,
}

#[tokio::main(flavor = "multi_thread", worker_threads = 10)]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();

    let args = Args::parse();
    let Some(listener) = args.listener.bind().await else {
        return Err(anyhow!("no listener specified"));
    };
    let runtime = Runtime::new(args.store.clone()).await?;
    let mut daemon = Daemon::new(runtime).await?;
    daemon.listen(listener?).await?;
    Ok(())
}
