use crate::error::Result;
use xenclient::{DomainConfig, XenClient};

pub struct Agent {
    client: XenClient,
    kernel_path: String,
    initrd_path: String,
    vcpus: u32,
    mem: u64,
}

impl Agent {
    pub fn new(kernel_path: String, initrd_path: String, vcpus: u32, mem: u64) -> Result<Agent> {
        let client = XenClient::open()?;
        Ok(Agent {
            client,
            kernel_path,
            initrd_path,
            vcpus,
            mem,
        })
    }

    pub fn launch(&mut self) -> Result<u32> {
        let config = DomainConfig {
            max_vcpus: self.vcpus,
            mem_mb: self.mem,
            kernel_path: self.kernel_path.as_str(),
            initrd_path: self.initrd_path.as_str(),
            cmdline: "debug elevator=noop",
        };
        Ok(self.client.create(&config)?)
    }
}
