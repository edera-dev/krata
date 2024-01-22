use crate::error::Result;
use crate::hypha_err;
use crate::shared::LaunchInfo;
use log::trace;
use nix::libc::dup2;
use nix::unistd::execve;
use oci_spec::image::{Config, ImageConfiguration};
use std::ffi::CString;
use std::fs;
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::os::linux::fs::MetadataExt;
use std::os::unix::fs::chroot;
use std::path::Path;
use sys_mount::{FilesystemType, Mount, MountFlags};
use walkdir::WalkDir;

const IMAGE_BLOCK_DEVICE_PATH: &str = "/dev/xvda";
const CONFIG_BLOCK_DEVICE_PATH: &str = "/dev/xvdb";

const IMAGE_MOUNT_PATH: &str = "/image";
const CONFIG_MOUNT_PATH: &str = "/config";
const OVERLAY_MOUNT_PATH: &str = "/overlay";

const OVERLAY_IMAGE_BIND_PATH: &str = "/overlay/image";
const OVERLAY_WORK_PATH: &str = "/overlay/work";
const OVERLAY_UPPER_PATH: &str = "/overlay/upper";

const NEW_ROOT_PATH: &str = "/newroot";
const IMAGE_CONFIG_JSON_PATH: &str = "/config/image/config.json";
const LAUNCH_CONFIG_JSON_PATH: &str = "/config/launch.json";

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
        let console = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/console")?;

        self.mount_squashfs_images()?;
        let config = self.parse_image_config()?;
        let launch = self.parse_launch_config()?;
        self.mount_new_root()?;
        self.nuke_initrd()?;
        self.bind_new_root()?;
        self.map_console(console)?;
        if let Some(cfg) = config.config() {
            self.run(cfg, &launch)?;
        } else {
            return hypha_err!("unable to determine what to execute, image config doesn't tell us");
        }
        Ok(())
    }

    fn mount_squashfs_images(&mut self) -> Result<()> {
        trace!("mounting squashfs images");
        let image_mount_path = Path::new(IMAGE_MOUNT_PATH);
        let config_mount_path = Path::new(CONFIG_MOUNT_PATH);
        self.mount_squashfs(Path::new(IMAGE_BLOCK_DEVICE_PATH), image_mount_path)?;
        self.mount_squashfs(Path::new(CONFIG_BLOCK_DEVICE_PATH), config_mount_path)?;
        Ok(())
    }

    fn mount_squashfs(&mut self, from: &Path, to: &Path) -> Result<()> {
        trace!("mounting squashfs image {:?} to {:?}", from, to);
        if !to.is_dir() {
            fs::create_dir(to)?;
        }
        Mount::builder()
            .fstype(FilesystemType::Manual("squashfs"))
            .flags(MountFlags::RDONLY)
            .mount(from, to)?;
        Ok(())
    }

    fn mount_new_root(&mut self) -> Result<()> {
        trace!("mounting new root");
        self.mount_overlay_tmpfs()?;
        self.bind_image_to_overlay_tmpfs()?;
        self.mount_overlay_to_new_root()?;
        std::env::set_current_dir(NEW_ROOT_PATH)?;
        trace!("mounted new root");
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

    fn mount_overlay_to_new_root(&mut self) -> Result<()> {
        fs::create_dir(NEW_ROOT_PATH)?;
        Mount::builder()
            .fstype(FilesystemType::Manual("overlay"))
            .flags(MountFlags::NOATIME)
            .data(&format!(
                "lowerdir={},upperdir={},workdir={}",
                OVERLAY_IMAGE_BIND_PATH, OVERLAY_UPPER_PATH, OVERLAY_WORK_PATH
            ))
            .mount(format!("overlayfs:{}", OVERLAY_MOUNT_PATH), NEW_ROOT_PATH)?;
        Ok(())
    }

    fn parse_image_config(&mut self) -> Result<ImageConfiguration> {
        trace!("parsing image config");
        let image_config_path = Path::new(IMAGE_CONFIG_JSON_PATH);
        let config = ImageConfiguration::from_file(image_config_path)?;
        Ok(config)
    }

    fn parse_launch_config(&mut self) -> Result<LaunchInfo> {
        trace!("parsing launch config");
        let launch_config = Path::new(LAUNCH_CONFIG_JSON_PATH);
        Ok(serde_json::from_str(&fs::read_to_string(launch_config)?)?)
    }

    fn nuke_initrd(&mut self) -> Result<()> {
        trace!("nuking initrd");
        let initrd_dev = fs::metadata("/")?.st_dev();
        for item in WalkDir::new("/")
            .same_file_system(true)
            .follow_links(false)
            .contents_first(true)
        {
            if item.is_err() {
                continue;
            }

            let item = item?;
            let metadata = match item.metadata() {
                Ok(value) => value,
                Err(_) => continue,
            };

            if metadata.st_dev() != initrd_dev {
                continue;
            }

            if metadata.is_symlink() || metadata.is_file() {
                let _ = fs::remove_file(item.path());
                trace!("deleting file {:?}", item.path());
            } else if metadata.is_dir() {
                let _ = fs::remove_dir(item.path());
                trace!("deleting directory {:?}", item.path());
            }
        }
        trace!("nuked initrd");
        Ok(())
    }

    fn bind_new_root(&mut self) -> Result<()> {
        trace!("binding new root");
        Mount::builder()
            .fstype(FilesystemType::Manual("none"))
            .flags(MountFlags::BIND)
            .mount(".", "/")?;
        trace!("chrooting into new root");
        chroot(".")?;
        trace!("setting root as current directory");
        std::env::set_current_dir("/")?;
        Ok(())
    }

    fn map_console(&mut self, console: File) -> Result<()> {
        trace!("map console");
        unsafe {
            dup2(console.as_raw_fd(), 0);
            dup2(console.as_raw_fd(), 1);
            dup2(console.as_raw_fd(), 2);
        }
        drop(console);
        Ok(())
    }

    fn run(&mut self, config: &Config, launch: &LaunchInfo) -> Result<()> {
        let mut cmd = match config.cmd() {
            None => vec![],
            Some(value) => value.clone(),
        };

        if launch.run.is_some() {
            cmd = launch.run.as_ref().unwrap().clone();
        }

        if cmd.is_empty() {
            cmd.push("/bin/sh".to_string());
        }

        trace!("running container command: {}", cmd.join(" "));
        let path = cmd.remove(0);
        let mut env = match config.env() {
            None => vec![],
            Some(value) => value.clone(),
        };
        env.push("HYPHA_CONTAINER=1".to_string());
        let path_cstr = CString::new(path)?;
        let cmd_cstr = ContainerInit::strings_as_cstrings(cmd)?;
        let env_cstr = ContainerInit::strings_as_cstrings(env)?;
        let mut working_dir = config
            .working_dir()
            .as_ref()
            .map(|x| x.to_string())
            .unwrap_or("/".to_string());

        if working_dir.is_empty() {
            working_dir = "/".to_string();
        }

        std::env::set_current_dir(&working_dir)?;
        execve(&path_cstr, &cmd_cstr, &env_cstr)?;
        Ok(())
    }

    fn strings_as_cstrings(values: Vec<String>) -> Result<Vec<CString>> {
        let mut results: Vec<CString> = vec![];
        for value in values {
            results.push(CString::new(value.as_bytes().to_vec())?);
        }
        Ok(results)
    }
}
