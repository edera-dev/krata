use crate::error::{HyphaError, Result};
use crate::image::cache::ImageCache;
use crate::image::name::ImageName;
use crate::image::{ImageCompiler, ImageInfo};
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;
use xenclient::{DomainConfig, DomainDisk, XenClient};

pub struct Controller {
    image_cache: ImageCache,
    image: String,
    client: XenClient,
    kernel_path: String,
    initrd_path: String,
    vcpus: u32,
    mem: u64,
}

impl Controller {
    pub fn new(
        store_path: String,
        kernel_path: String,
        initrd_path: String,
        image: String,
        vcpus: u32,
        mem: u64,
    ) -> Result<Controller> {
        let mut image_cache_path = PathBuf::from(store_path);
        image_cache_path.push("cache");
        fs::create_dir_all(&image_cache_path)?;

        let client = XenClient::open()?;
        image_cache_path.push("image");
        fs::create_dir_all(&image_cache_path)?;
        let image_cache = ImageCache::new(&image_cache_path)?;
        Ok(Controller {
            image_cache,
            image,
            client,
            kernel_path,
            initrd_path,
            vcpus,
            mem,
        })
    }

    fn compile(&mut self) -> Result<ImageInfo> {
        let image = ImageName::parse(&self.image)?;
        let compiler = ImageCompiler::new(&self.image_cache)?;
        compiler.compile(&image)
    }

    pub fn launch(&mut self) -> Result<u32> {
        let uuid = Uuid::new_v4();
        let name = format!("hypha-{uuid}");
        let image_info = self.compile()?;
        let squashfs_path = image_info
            .squashfs
            .to_str()
            .ok_or_else(|| HyphaError::new("failed to convert squashfs path to string"))?;
        let config = DomainConfig {
            backend_domid: 0,
            name: &name,
            max_vcpus: self.vcpus,
            mem_mb: self.mem,
            kernel_path: self.kernel_path.as_str(),
            initrd_path: self.initrd_path.as_str(),
            cmdline: "elevator=noop",
            disks: vec![DomainDisk {
                vdev: "xvda",
                pdev: squashfs_path,
                writable: false,
            }],
        };
        Ok(self.client.create(&config)?)
    }
}
