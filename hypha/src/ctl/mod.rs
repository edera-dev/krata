pub mod cfgblk;

use crate::autoloop::AutoLoop;
use crate::ctl::cfgblk::ConfigBlock;
use crate::error::Result;
use crate::image::cache::ImageCache;
use crate::image::name::ImageName;
use crate::image::{ImageCompiler, ImageInfo};
use loopdev::LoopControl;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;
use xenclient::{DomainConfig, DomainDisk, XenClient};

pub struct Controller {
    image_cache: ImageCache,
    autoloop: AutoLoop,
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
            autoloop: AutoLoop::new(LoopControl::open()?),
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
        let cfgblk = ConfigBlock::new(&uuid, &image_info)?;
        cfgblk.build()?;

        let image_squashfs_path = image_info.image_squashfs.clone();
        let cfgblk_squashfs_path = cfgblk.file.clone();

        let image_squashfs_loop = self.autoloop.loopify(&image_squashfs_path)?;
        let cfgblk_squashfs_loop = self.autoloop.loopify(&cfgblk_squashfs_path)?;

        let config = DomainConfig {
            backend_domid: 0,
            name: &name,
            max_vcpus: self.vcpus,
            mem_mb: self.mem,
            kernel_path: self.kernel_path.as_str(),
            initrd_path: self.initrd_path.as_str(),
            cmdline: "quiet elevator=noop",
            disks: vec![
                DomainDisk {
                    vdev: "xvda",
                    block: &image_squashfs_loop,
                    writable: false,
                },
                DomainDisk {
                    vdev: "xvdb",
                    block: &cfgblk_squashfs_loop,
                    writable: false,
                },
            ],
        };
        match self.client.create(&config) {
            Ok(domid) => Ok(domid),
            Err(error) => {
                let _ = self.autoloop.unloop(&image_squashfs_loop.path);
                let _ = self.autoloop.unloop(&cfgblk_squashfs_loop.path);
                let _ = fs::remove_dir(&cfgblk.dir);
                Err(error.into())
            }
        }
    }
}
