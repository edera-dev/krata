use super::{BlockDeviceRef, BlockDeviceResult, DeviceConfig, DeviceDescription, XenTransaction};
use crate::{
    error::{Error, Result},
    util::vbd_blkidx_to_disk_name,
};

pub struct VbdDeviceConfig {
    backend_type: String,
    removable: bool,
    bootable: bool,
    writable: bool,
    discard: bool,
    trusted: bool,
    block_device: Option<BlockDeviceRef>,
}

impl Default for VbdDeviceConfig {
    fn default() -> Self {
        Self::new()
    }
}

impl VbdDeviceConfig {
    pub fn new() -> Self {
        Self {
            backend_type: "vbd".to_string(),
            removable: false,
            bootable: true,
            writable: false,
            discard: false,
            trusted: true,
            block_device: None,
        }
    }

    pub fn backend_type(&mut self, backend_type: impl AsRef<str>) -> &mut Self {
        self.backend_type = backend_type.as_ref().to_string();
        self
    }

    pub fn removable(&mut self, removable: bool) -> &mut Self {
        self.removable = removable;
        self
    }

    pub fn bootable(&mut self, bootable: bool) -> &mut Self {
        self.bootable = bootable;
        self
    }

    pub fn writable(&mut self, writable: bool) -> &mut Self {
        self.writable = writable;
        self
    }

    pub fn discard(&mut self, discard: bool) -> &mut Self {
        self.discard = discard;
        self
    }

    pub fn trusted(&mut self, trusted: bool) -> &mut Self {
        self.trusted = trusted;
        self
    }

    pub fn block_device(&mut self, block_device: BlockDeviceRef) -> &mut Self {
        self.block_device = Some(block_device);
        self
    }

    pub fn done(self) -> Self {
        self
    }
}

#[async_trait::async_trait]
impl DeviceConfig for VbdDeviceConfig {
    type Result = BlockDeviceResult;

    async fn add_to_transaction(&self, tx: &XenTransaction) -> Result<BlockDeviceResult> {
        let id = tx.assign_next_devid().await?;
        let idx = tx.assign_next_blkidx().await?;
        let vdev = vbd_blkidx_to_disk_name(idx)?;
        let block_device = self
            .block_device
            .as_ref()
            .ok_or_else(|| Error::ParameterMissing("block device"))?;

        let mut device = DeviceDescription::new("vbd", &self.backend_type);
        device
            .add_backend_item("online", 1)
            .add_backend_bool("removable", self.removable)
            .add_backend_bool("bootable", self.bootable)
            .add_backend_item("type", "phy")
            .add_backend_item("device-type", "disk")
            .add_backend_item("discard-enable", self.discard)
            .add_backend_item("specification", "xen")
            .add_backend_item("physical-device-path", &block_device.path)
            .add_backend_item("mode", if self.writable { "w" } else { "r" })
            .add_backend_item(
                "physical-device",
                format!("{:02x}:{:02x}", block_device.major, block_device.minor),
            )
            .add_backend_item("dev", &vdev)
            .add_backend_item("state", 1);

        // we should use standard virtual-device support for first few block devices.
        // the kernel warns when you use ext for indexes 5 or less, due to
        // potential id overlapping.
        let (vdev, vd_key) = if idx <= 5 {
            // shift by 4 as partition count is 16
            ((202 << 8) | (idx as u64 * 16u64), "virtual-device")
        } else {
            // this is silly but 256 is the number of partitions
            // multiply the index by that to get the actual id
            ((1u64 << 28u64) + (idx as u64) * 256, "virtual-device-ext")
        };

        device
            .add_frontend_item(vd_key, vdev)
            .add_frontend_item("state", 1)
            .add_frontend_item("device-type", "disk")
            .add_frontend_bool("trusted", self.trusted)
            .add_frontend_item("protocol", "x86_64-abi")
            .add_frontend_item("x-index", idx);

        tx.add_device(id, device).await?;

        Ok(BlockDeviceResult { id, idx })
    }
}
