use anyhow::Result;
use env_logger::Env;
use kratart::channel::ChannelService;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let (service, _, mut receiver) = ChannelService::new("krata-channel".to_string(), None).await?;
    let task = service.launch().await?;

    loop {
        let Some((id, data)) = receiver.recv().await else {
            break;
        };

        println!("domain {} = {:?}", id, data);
    }

    task.abort();

    Ok(())
}
