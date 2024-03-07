use anyhow::{anyhow, Result};
use loopdev::{LoopControl, LoopDevice};
use xenclient::BlockDeviceRef;

pub struct AutoLoop {
    control: LoopControl,
}

impl AutoLoop {
    pub fn new(control: LoopControl) -> AutoLoop {
        AutoLoop { control }
    }

    pub fn loopify(&self, file: &str) -> Result<BlockDeviceRef> {
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

    pub fn unloop(&self, device: &str) -> Result<()> {
        let device = LoopDevice::open(device)?;
        device.detach()?;
        Ok(())
    }
}
