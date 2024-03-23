use std::env::args;

use xenstore::error::Result;
use xenstore::{XsdClient, XsdInterface};

async fn list_recursive(client: &XsdClient, path: &str) -> Result<()> {
    let mut pending = vec![path.to_string()];

    while let Some(ref path) = pending.pop() {
        let children = client.list(path).await?;
        for child in children {
            let full = format!("{}/{}", if path == "/" { "" } else { path }, child);
            let value = client
                .read_string(full.as_str())
                .await?
                .expect("expected value");
            println!("{} = {:?}", full, value,);
            pending.push(full);
        }
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();
    let client = XsdClient::open().await?;
    loop {
        list_recursive(&client, "/").await?;
        if args().nth(1).unwrap_or("none".to_string()) != "stress" {
            break;
        }
    }
    Ok(())
}
