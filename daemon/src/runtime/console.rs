use anyhow::Result;
use tokio::fs::File;

pub struct XenConsole {
    pub read_handle: File,
    pub write_handle: File,
}

impl XenConsole {
    pub async fn new(tty: &str) -> Result<XenConsole> {
        let read_handle = File::options().read(true).write(false).open(tty).await?;
        let write_handle = File::options().read(false).write(true).open(tty).await?;
        Ok(XenConsole {
            read_handle,
            write_handle,
        })
    }
}
