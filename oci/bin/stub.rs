use anyhow::{anyhow, Result};
use krata::dial::ControlDialAddress;
use kratactl::{client::ControlClientProvider, console::StdioConsoleStream};
use std::env::args;
use std::str::FromStr;

#[tokio::main]
async fn main() -> Result<()> {
    let Some(guest) = args().nth(1) else {
        return Err(anyhow!("guest id not specified"));
    };
    let address = ControlDialAddress::from_str("unix:///var/lib/krata/daemon.socket")?;
    let mut client = ControlClientProvider::dial(address).await?;
    let input = StdioConsoleStream::stdin_stream(guest).await;
    let output = client.console_data(input).await?.into_inner();
    StdioConsoleStream::stdout(output).await?;
    Ok(())
}
