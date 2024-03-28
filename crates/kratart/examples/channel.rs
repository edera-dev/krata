use anyhow::Result;
use env_logger::Env;
use kratart::chan::KrataChannelService;
use xenevtchn::EventChannel;
use xengnt::GrantTab;
use xenstore::XsdClient;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let mut krata = KrataChannelService::new(
        EventChannel::open().await?,
        XsdClient::open().await?,
        GrantTab::open()?,
    )?;
    krata.watch().await?;
    Ok(())
}
