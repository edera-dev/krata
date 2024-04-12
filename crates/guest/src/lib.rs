use std::{os::raw::c_int, time::Duration};

use anyhow::Result;
use tokio::time::sleep;
use xenstore::{XsdClient, XsdInterface};

pub mod background;
pub mod childwait;
pub mod init;
pub mod metrics;

pub async fn death(code: c_int) -> Result<()> {
    let store = XsdClient::open().await?;
    store
        .write_string("krata/guest/exit-code", &code.to_string())
        .await?;
    drop(store);
    loop {
        sleep(Duration::from_secs(1)).await;
    }
}
