use crate::error::{HyphaError, Result};
use std::fs::{read_dir, DirEntry};
use xenclient::create::DomainConfig;
use xenclient::XenClient;

pub struct Agent {
    client: XenClient,
}

impl Agent {
    pub fn new() -> Result<Agent> {
        let client = XenClient::open()?;
        Ok(Agent { client })
    }

    pub fn launch(&mut self) -> Result<u32> {
        let kernel_path = self.find_boot_path("vmlinuz-")?;
        let initrd_path = self.find_boot_path("initrd.img-")?;

        let config = DomainConfig {
            max_vcpus: 1,
            mem_mb: 512,
            kernel_path,
            initrd_path,
            cmdline: "debug elevator=noop".to_string(),
        };
        Ok(self.client.create(config)?)
    }

    fn find_boot_path(&self, prefix: &str) -> Result<String> {
        let vmlinuz = read_dir("/boot")?
            .filter(|x| x.is_ok())
            .map(|x| x.unwrap())
            .filter(|x| {
                x.file_name()
                    .to_str()
                    .ok_or(HyphaError::new("invalid direntry"))
                    .map(|x| x.starts_with(prefix))
                    .unwrap_or(false)
            })
            .collect::<Vec<DirEntry>>();
        Ok(vmlinuz
            .first()
            .ok_or(HyphaError::new("unable to find suitable image"))?
            .path()
            .to_str()
            .ok_or(HyphaError::new("invalid direntry"))?
            .to_string())
    }
}
