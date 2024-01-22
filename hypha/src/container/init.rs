use crate::error::Result;
use oci_spec::image::ImageConfiguration;
use std::fs;
use std::path::Path;
use sys_mount::{FilesystemType, Mount, MountFlags};

const IMAGE_BLOCK_DEVICE_PATH: &str = "/dev/xvda";
const CONFIG_BLOCK_DEVICE_PATH: &str = "/dev/xvdb";

const IMAGE_MOUNT_PATH: &str = "/image";
const CONFIG_MOUNT_PATH: &str = "/config";
const OVERLAY_MOUNT_PATH: &str = "/overlay";

const OVERLAY_IMAGE_BIND_PATH: &str = "/overlay/image";
const OVERLAY_WORK_PATH: &str = "/overlay/work";
const OVERLAY_UPPER_PATH: &str = "/overlay/upper";

const PIVOT_PATH: &str = "/pivot";

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
        self.mount_early()?;
        let config = self.parse_image_config()?;
        self.mount_late()?;

        if let Some(cfg) = config.config() {
            if let Some(cmd) = cfg.cmd() {
                println!("image command: {:?}", cmd);
            }
        }
        Ok(())
    }

    fn mount_early(&mut self) -> Result<()> {
        let image_mount_path = Path::new(IMAGE_MOUNT_PATH);
        let config_mount_path = Path::new(CONFIG_MOUNT_PATH);
        self.mount_squashfs(Path::new(IMAGE_BLOCK_DEVICE_PATH), image_mount_path)?;
        self.mount_squashfs(Path::new(CONFIG_BLOCK_DEVICE_PATH), config_mount_path)?;
        Ok(())
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

    fn mount_late(&mut self) -> Result<()> {
        self.mount_overlay_tmpfs()?;
        self.bind_image_to_overlay_tmpfs()?;
        self.mount_overlay_to_pivot()?;
        Ok(())
    }

    fn mount_overlay_tmpfs(&mut self) -> Result<()> {
        fs::create_dir(OVERLAY_MOUNT_PATH)?;
        Mount::builder()
            .fstype(FilesystemType::Manual("tmpfs"))
            .mount("tmpfs", OVERLAY_MOUNT_PATH)?;
        fs::create_dir(OVERLAY_UPPER_PATH)?;
        fs::create_dir(OVERLAY_WORK_PATH)?;
        Ok(())
    }

    fn bind_image_to_overlay_tmpfs(&mut self) -> Result<()> {
        fs::create_dir(OVERLAY_IMAGE_BIND_PATH)?;
        Mount::builder()
            .fstype(FilesystemType::Manual("none"))
            .flags(MountFlags::BIND | MountFlags::RDONLY)
            .mount(IMAGE_MOUNT_PATH, OVERLAY_IMAGE_BIND_PATH)?;
        Ok(())
    }

    fn mount_overlay_to_pivot(&mut self) -> Result<()> {
        fs::create_dir(PIVOT_PATH)?;
        Mount::builder()
            .fstype(FilesystemType::Manual("overlay"))
            .flags(MountFlags::NOATIME)
            .data(&format!(
                "lowerdir={},upperdir={},workdir={}",
                OVERLAY_IMAGE_BIND_PATH, OVERLAY_UPPER_PATH, OVERLAY_WORK_PATH
            ))
            .mount(format!("overlayfs:{}", OVERLAY_MOUNT_PATH), PIVOT_PATH)?;
        Ok(())
    }

    fn parse_image_config(&mut self) -> Result<ImageConfiguration> {
        let image_config_path = Path::new(IMAGE_CONFIG_JSON_PATH);
        let config = ImageConfiguration::from_file(image_config_path)?;
        Ok(config)
    }
}
