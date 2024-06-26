use std::{sync::Arc, time::Duration};

use anyhow::{anyhow, Result};
use krataloopdev::{LoopControl, LoopDevice};
use log::debug;
use tokio::time::sleep;
use xenclient::BlockDeviceRef;

#[derive(Clone)]
pub struct AutoLoop {
    control: Arc<LoopControl>,
}

impl AutoLoop {
    pub fn new(control: LoopControl) -> AutoLoop {
        AutoLoop {
            control: Arc::new(control),
        }
    }

    pub fn loopify(&self, file: &str) -> Result<BlockDeviceRef> {
        debug!("creating loop for file {}", file);
        let device = self.control.next_free()?;
        device.with().read_only(true).attach(file)?;
        let path = device
            .path()
            .ok_or(anyhow!("unable to get loop device path"))?
            .to_str()
            .ok_or(anyhow!("unable to convert loop device path to string",))?
            .to_string();
        let major = device.major()?;
        let minor = device.minor()?;
        Ok(BlockDeviceRef { path, major, minor })
    }

    pub async fn unloop(&self, device: &str) -> Result<()> {
        let device = LoopDevice::open(device)?;
        device.detach()?;
        sleep(Duration::from_millis(200)).await;
        Ok(())
    }
}
