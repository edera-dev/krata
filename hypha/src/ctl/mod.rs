use crate::error::Result;
use crate::image::ImageCompiler;
use ocipkg::ImageName;
use xenclient::{DomainConfig, XenClient};

pub struct Controller {
    client: XenClient,
    kernel_path: String,
    initrd_path: String,
    vcpus: u32,
    mem: u64,
    image: String,
}

impl Controller {
    pub fn new(
        kernel_path: String,
        initrd_path: String,
        image: String,
        vcpus: u32,
        mem: u64,
    ) -> Result<Controller> {
        let client = XenClient::open()?;
        Ok(Controller {
            client,
            kernel_path,
            initrd_path,
            image,
            vcpus,
            mem,
        })
    }

    pub fn compile(&mut self) -> Result<()> {
        let image = ImageName::parse(&self.image)?;
        let compiler = ImageCompiler::new()?;
        let _squashfs = compiler.compile(&image)?;
        Ok(())
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
