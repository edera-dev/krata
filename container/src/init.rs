use anyhow::{anyhow, Result};
use futures::stream::TryStreamExt;
use hypha::{LaunchInfo, LaunchNetwork};
use ipnetwork::IpNetwork;
use log::{trace, warn};
use nix::libc::{c_int, dup2, wait};
use nix::unistd::{execve, fork, ForkResult, Pid};
use oci_spec::image::{Config, ImageConfiguration};
use std::ffi::{CStr, CString};
use std::fs;
use std::fs::{File, OpenOptions, Permissions};
use std::net::Ipv4Addr;
use std::os::fd::AsRawFd;
use std::os::linux::fs::MetadataExt;
use std::os::unix::fs::{chroot, PermissionsExt};
use std::path::Path;
use std::ptr::addr_of_mut;
use std::thread::sleep;
use std::time::Duration;
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

const SYS_PATH: &str = "/sys";
const PROC_PATH: &str = "/proc";
const DEV_PATH: &str = "/dev";

const NEW_ROOT_PATH: &str = "/newroot";
const NEW_ROOT_SYS_PATH: &str = "/newroot/sys";
const NEW_ROOT_PROC_PATH: &str = "/newroot/proc";
const NEW_ROOT_DEV_PATH: &str = "/newroot/dev";

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

    pub async fn init(&mut self) -> Result<()> {
        self.early_init()?;

        trace!("opening console descriptor");
        match OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/console")
        {
            Ok(console) => self.map_console(console)?,
            Err(error) => warn!("failed to open console: {}", error),
        }

        self.mount_squashfs_images()?;
        let config = self.parse_image_config()?;
        let launch = self.parse_launch_config()?;
        self.mount_new_root()?;
        self.nuke_initrd()?;
        self.bind_new_root()?;

        if let Some(network) = &launch.network {
            if let Err(error) = self.network_setup(network).await {
                warn!("failed to initialize network: {}", error);
            }
        }

        if let Some(cfg) = config.config() {
            self.run(cfg, &launch)?;
        } else {
            return Err(anyhow!(
                "unable to determine what to execute, image config doesn't tell us"
            ));
        }
        Ok(())
    }

    fn early_init(&mut self) -> Result<()> {
        trace!("early init");
        self.create_dir("/dev", Some(0o0755))?;
        self.create_dir("/proc", None)?;
        self.create_dir("/sys", None)?;
        self.create_dir("/root", Some(0o0700))?;
        self.create_dir("/tmp", None)?;
        self.mount_kernel_fs("devtmpfs", "/dev", "mode=0755")?;
        self.mount_kernel_fs("proc", "/proc", "")?;
        self.mount_kernel_fs("sysfs", "/sys", "")?;
        Ok(())
    }

    fn create_dir(&mut self, path: &str, mode: Option<u32>) -> Result<()> {
        let path = Path::new(path);
        if !path.is_dir() {
            trace!("creating directory {:?}", path);
            fs::create_dir(path)?;
        }
        if let Some(mode) = mode {
            let permissions = Permissions::from_mode(mode);
            trace!("setting directory {:?} permissions to {:?}", path, mode);
            fs::set_permissions(path, permissions)?;
        }
        Ok(())
    }

    fn mount_kernel_fs(&mut self, fstype: &str, path: &str, data: &str) -> Result<()> {
        let metadata = fs::metadata(path)?;
        if metadata.st_dev() == fs::metadata("/")?.st_dev() {
            trace!("mounting kernel fs {} to {}", fstype, path);
            Mount::builder()
                .fstype(FilesystemType::Manual(fstype))
                .flags(MountFlags::NOEXEC | MountFlags::NOSUID)
                .data(data)
                .mount(fstype, path)?;
        }
        Ok(())
    }

    fn map_console(&mut self, console: File) -> Result<()> {
        trace!("mapping console");
        unsafe {
            dup2(console.as_raw_fd(), 0);
            dup2(console.as_raw_fd(), 1);
            dup2(console.as_raw_fd(), 2);
        }
        drop(console);
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

    fn mount_move_subtree(&mut self, from: &Path, to: &Path) -> Result<()> {
        trace!("moving subtree {:?} to {:?}", from, to);
        if !to.is_dir() {
            fs::create_dir(to)?;
        }
        Mount::builder()
            .fstype(FilesystemType::Manual("none"))
            .flags(MountFlags::MOVE)
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
        self.mount_move_subtree(Path::new(SYS_PATH), Path::new(NEW_ROOT_SYS_PATH))?;
        self.mount_move_subtree(Path::new(PROC_PATH), Path::new(NEW_ROOT_PROC_PATH))?;
        self.mount_move_subtree(Path::new(DEV_PATH), Path::new(NEW_ROOT_DEV_PATH))?;
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

    async fn network_setup(&mut self, network: &LaunchNetwork) -> Result<()> {
        trace!(
            "setting up network with link {} and ipv4 {}",
            network.link,
            network.ipv4
        );

        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let ip: IpNetwork = network.ipv4.parse()?;

        let mut links = handle
            .link()
            .get()
            .match_name(network.link.clone())
            .execute();
        if let Some(link) = links.try_next().await? {
            handle
                .address()
                .add(link.header.index, ip.ip(), ip.prefix())
                .execute()
                .await?;

            handle
                .link()
                .set(link.header.index)
                .arp(false)
                .up()
                .execute()
                .await?;

            handle
                .route()
                .add()
                .v4()
                .destination_prefix(Ipv4Addr::new(0, 0, 0, 0), 0)
                .output_interface(link.header.index)
                .execute()
                .await?;
        } else {
            warn!("unable to find link named {}", network.link);
        }
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

        let path = cmd.remove(0);
        let mut env = match config.env() {
            None => vec![],
            Some(value) => value.clone(),
        };
        env.push("HYPHA_CONTAINER=1".to_string());
        if let Some(extra_env) = &launch.env {
            env.extend_from_slice(extra_env.as_slice());
        }

        trace!("running container command: {}", cmd.join(" "));

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
        self.fork_and_exec(&path_cstr, cmd_cstr, env_cstr)?;
        Ok(())
    }

    fn strings_as_cstrings(values: Vec<String>) -> Result<Vec<CString>> {
        let mut results: Vec<CString> = vec![];
        for value in values {
            results.push(CString::new(value.as_bytes().to_vec())?);
        }
        Ok(results)
    }

    fn fork_and_exec(&mut self, path: &CStr, cmd: Vec<CString>, env: Vec<CString>) -> Result<()> {
        match unsafe { fork()? } {
            ForkResult::Parent { child } => self.background(child),
            ForkResult::Child => {
                execve(path, &cmd, &env)?;
                Ok(())
            }
        }
    }

    fn background(&mut self, executed: Pid) -> Result<()> {
        loop {
            let mut status: c_int = 0;
            let pid = unsafe { wait(addr_of_mut!(status)) };
            if executed.as_raw() == pid {
                return self.death(status);
            }
        }
    }

    fn death(&mut self, code: c_int) -> Result<()> {
        println!("[hypha] container process exited: status = {}", code);
        println!("[hypha] looping forever");
        loop {
            sleep(Duration::from_secs(1));
        }
    }
}
