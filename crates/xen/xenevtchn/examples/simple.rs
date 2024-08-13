use xenevtchn::error::Result;
use xenevtchn::EventChannelService;

#[tokio::main]
async fn main() -> Result<()> {
    let channel = EventChannelService::open().await?;
    println!("channel opened");
    let port = channel.bind_unbound_port(0).await?;
    println!("port: {}", port);
    channel.unbind(port).await?;
    Ok(())
}
