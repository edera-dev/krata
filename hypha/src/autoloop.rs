use crate::error::{HyphaError, Result};
use loopdev::{LoopControl, LoopDevice};
use std::path::Path;
use xenclient::BlockDeviceRef;

pub struct AutoLoop {
    control: LoopControl,
}

impl AutoLoop {
    pub(crate) fn new(control: LoopControl) -> AutoLoop {
        AutoLoop { control }
    }

    pub fn loopify(&self, file: &Path) -> Result<BlockDeviceRef> {
        let device = self.control.next_free()?;
        device.with().read_only(true).attach(file)?;
        let path = device
            .path()
            .ok_or(HyphaError::new("unable to get loop device path"))?
            .to_str()
            .ok_or(HyphaError::new(
                "unable to convert loop device path to string",
            ))?
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
