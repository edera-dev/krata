pub mod cfgblk;

use crate::autoloop::AutoLoop;
use crate::ctl::cfgblk::ConfigBlock;
use crate::error::{HyphaError, Result};
use crate::image::cache::ImageCache;
use crate::image::name::ImageName;
use crate::image::{ImageCompiler, ImageInfo};
use loopdev::LoopControl;
use std::fs;
use std::path::PathBuf;
use uuid::Uuid;
use xenclient::{DomainConfig, DomainDisk, XenClient};
use xenstore::client::{XsdClient, XsdInterface};

pub struct Controller {
    image_cache: ImageCache,
    autoloop: AutoLoop,
    client: XenClient,
    kernel_path: String,
    initrd_path: String,
}

impl Controller {
    pub fn new(store_path: String, kernel_path: String, initrd_path: String) -> Result<Controller> {
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
            client,
            kernel_path,
            initrd_path,
        })
    }

    fn compile(&mut self, image: &str) -> Result<ImageInfo> {
        let image = ImageName::parse(image)?;
        let compiler = ImageCompiler::new(&self.image_cache)?;
        compiler.compile(&image)
    }

    pub fn launch(&mut self, image: &str, vcpus: u32, mem: u64) -> Result<u32> {
        let uuid = Uuid::new_v4();
        let name = format!("hypha-{uuid}");
        let image_info = self.compile(image)?;
        let cfgblk = ConfigBlock::new(&uuid, &image_info)?;
        cfgblk.build()?;

        let image_squashfs_path = image_info.image_squashfs.clone();
        let cfgblk_squashfs_path = cfgblk.file.clone();

        let image_squashfs_loop = self.autoloop.loopify(&image_squashfs_path)?;
        let cfgblk_squashfs_loop = self.autoloop.loopify(&cfgblk_squashfs_path)?;

        let config = DomainConfig {
            backend_domid: 0,
            name: &name,
            max_vcpus: vcpus,
            mem_mb: mem,
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
            extra_keys: vec![
                ("hypha/uuid".to_string(), uuid.to_string()),
                (
                    "hypha/loops".to_string(),
                    format!(
                        "{},{}",
                        &image_squashfs_loop.path, &cfgblk_squashfs_loop.path
                    ),
                ),
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

    pub fn destroy(&mut self, domid: u32) -> Result<Uuid> {
        let mut store = XsdClient::open()?;
        let dom_path = store.get_domain_path(domid)?;
        let uuid = match store.read_string_optional(format!("{}/hypha/uuid", dom_path).as_str())? {
            None => {
                return Err(HyphaError::new(&format!(
                    "domain {} was not found or not created by hypha",
                    domid
                )))
            }
            Some(value) => value,
        };
        if uuid.is_empty() {
            return Err(HyphaError::new(
                "unable to find hypha uuid based on the domain",
            ));
        }
        let uuid = Uuid::parse_str(&uuid)?;
        let loops = store.read_string(format!("{}/hypha/loops", dom_path).as_str())?;
        let loops = loops
            .split(',')
            .map(|x| x.to_string())
            .collect::<Vec<String>>();
        self.client.destroy(domid)?;
        for lop in &loops {
            self.autoloop.unloop(lop)?;
        }
        Ok(uuid)
    }
}
