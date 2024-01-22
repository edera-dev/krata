use crate::error::Result;
use oci_spec::image::ImageConfiguration;
use std::fs;
use std::path::Path;
use sys_mount::{FilesystemType, Mount, MountFlags};

const IMAGE_BLOCK_DEVICE_PATH: &str = "/dev/xvda";
const CONFIG_BLOCK_DEVICE_PATH: &str = "/dev/xvdb";

const IMAGE_MOUNT_PATH: &str = "/image";
const CONFIG_MOUNT_PATH: &str = "/config";

const IMAGE_CONFIG_JSON_PATH: &str = "/config/image/config.json";

pub struct ContainerInit {}

impl Default for ContainerInit {
    fn default() -> Self {
        Self::new()
    }
}

impl ContainerInit {
    pub fn new() -> ContainerInit {
        ContainerInit {}
    }

    pub fn init(&mut self) -> Result<()> {
        self.prepare_mounts()?;
        let config = self.parse_image_config()?;
        if let Some(cfg) = config.config() {
            if let Some(cmd) = cfg.cmd() {
                println!("image command: {:?}", cmd);
            }
        }
        Ok(())
    }

    fn prepare_mounts(&mut self) -> Result<()> {
        let image_mount_path = Path::new(IMAGE_MOUNT_PATH);
        let config_mount_path = Path::new(CONFIG_MOUNT_PATH);
        self.mount_squashfs(Path::new(IMAGE_BLOCK_DEVICE_PATH), image_mount_path)?;
        self.mount_squashfs(Path::new(CONFIG_BLOCK_DEVICE_PATH), config_mount_path)?;
        Ok(())
    }

    fn parse_image_config(&mut self) -> Result<ImageConfiguration> {
        let image_config_path = Path::new(IMAGE_CONFIG_JSON_PATH);
        let config = ImageConfiguration::from_file(image_config_path)?;
        Ok(config)
    }

    fn mount_squashfs(&mut self, from: &Path, to: &Path) -> Result<()> {
        if !to.is_dir() {
            fs::create_dir(to)?;
        }
        Mount::builder()
            .fstype(FilesystemType::Manual("squashfs"))
            .flags(MountFlags::RDONLY)
            .mount(from, to)?;
        Ok(())
    }
}
