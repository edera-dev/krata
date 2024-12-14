use std::time::Duration;

use tokio::{
    select,
    time::{sleep, timeout},
};
use xenstore::{XsdClient, XsdInterface};

use crate::error::{Error, Result};

pub struct DeviceLocator {
    pub frontend_domid: u32,
    pub backend_domid: u32,
    pub frontend_type: String,
    pub backend_type: String,
    pub device_id: u64,
}

impl DeviceLocator {
    pub fn new(
        frontend_domid: u32,
        backend_domid: u32,
        frontend_type: String,
        backend_type: String,
        device_id: u64,
    ) -> Self {
        DeviceLocator {
            frontend_domid,
            backend_domid,
            frontend_type,
            backend_type,
            device_id,
        }
    }

    pub fn frontend_state_path(&self) -> String {
        format!(
            "/local/domain/{}/device/{}/{}/state",
            self.frontend_domid, self.frontend_type, self.device_id
        )
    }

    pub fn backend_state_path(&self) -> String {
        format!(
            "/local/domain/{}/backend/{}/{}/{}/state",
            self.backend_domid, self.backend_type, self.frontend_domid, self.device_id
        )
    }
}

pub struct DeviceStateWaiter {
    devices: Vec<DeviceLocator>,
    xsd: XsdClient,
}

impl DeviceStateWaiter {
    pub fn new(xsd: XsdClient) -> Self {
        DeviceStateWaiter {
            devices: vec![],
            xsd,
        }
    }

    pub fn add_device(&mut self, device: DeviceLocator) -> &mut DeviceStateWaiter {
        self.devices.push(device);
        self
    }

    async fn check_states(xsd: &XsdClient, state_paths: &[String], desired: u32) -> Result<bool> {
        let mut ready = 0;
        for state_path in state_paths {
            let Some(state_text) = xsd.read_string(state_path).await? else {
                return Err(Error::DevStateWaitError(format!(
                    "state path '{}' did not exist",
                    state_path
                )));
            };

            let Some(state_value) = state_text.parse::<u32>().ok() else {
                return Err(Error::DevStateWaitError(format!(
                    "state path '{}' did not have a valid value",
                    state_path
                )));
            };

            if state_value > desired {
                return Err(Error::DevStateWaitError(format!(
                    "state path '{}' had a state of {} which is greater than {}",
                    state_path, state_value, desired
                )));
            }

            if state_value == desired {
                ready += 1;
            }
        }
        Ok(ready == state_paths.len())
    }

    async fn do_wait(self, desired: u32) -> Result<()> {
        let mut watch = self.xsd.create_multi_watch().await?;
        let mut state_paths = Vec::new();
        for device in self.devices {
            let state_path = device.backend_state_path();
            self.xsd.bind_watch_id(watch.id, &state_path).await?;
            state_paths.push(state_path);
        }

        loop {
            if DeviceStateWaiter::check_states(&self.xsd, &state_paths, desired).await? {
                break;
            }

            select! {
                _update = watch.receiver.recv() => {},
                _timeout = sleep(Duration::from_millis(250)) => {},
            }
        }
        Ok(())
    }

    pub async fn wait(self, desired: u32, deadline: Duration) -> Result<()> {
        if let Some(err) = timeout(deadline, self.do_wait(desired)).await.err() {
            return Err(Error::DevStateWaitError(format!(
                "took too long for devices to be ready: {}",
                err
            )));
        }
        Ok(())
    }
}
