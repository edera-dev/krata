use anyhow::Result;

use krata::v1::control::{DeviceInfo, ListDevicesReply, ListDevicesRequest};

use crate::devices::DaemonDeviceManager;

pub struct ListDevicesRpc {
    devices: DaemonDeviceManager,
}

impl ListDevicesRpc {
    pub fn new(devices: DaemonDeviceManager) -> Self {
        Self { devices }
    }

    pub async fn process(self, _request: ListDevicesRequest) -> Result<ListDevicesReply> {
        let mut devices = Vec::new();
        let state = self.devices.copy().await?;
        for (name, state) in state {
            devices.push(DeviceInfo {
                name,
                claimed: state.owner.is_some(),
                owner: state.owner.map(|x| x.to_string()).unwrap_or_default(),
            });
        }
        Ok(ListDevicesReply { devices })
    }
}
