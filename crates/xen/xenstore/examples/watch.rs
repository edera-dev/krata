use std::env::args;
use xenstore::error::Result;
use xenstore::XsdClient;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let path = args().nth(1).unwrap_or("/local/domain".to_string());
    let client = XsdClient::open().await?;
    let mut handle = client.watch(&path).await?;
    let mut count = 0;
    loop {
        let Some(event) = handle.receiver.recv().await else {
            break;
        };
        println!("{}", event);
        count += 1;
        if count >= 3 {
            break;
        }
    }
    Ok(())
}
