use anyhow::{anyhow, Result};
use cgroups_rs::{Cgroup, CgroupPid};
use futures::stream::TryStreamExt;
use ipnetwork::IpNetwork;
use krata::ethtool::EthtoolHandle;
use krata::idm::client::IdmClient;
use krata::launchcfg::{LaunchInfo, LaunchNetwork};
use libc::{sethostname, setsid, TIOCSCTTY};
use log::{trace, warn};
use nix::ioctl_write_int_bad;
use nix::unistd::{dup2, execve, fork, ForkResult, Pid};
use oci_spec::image::{Config, ImageConfiguration};
use path_absolutize::Absolutize;
use std::collections::HashMap;
use std::ffi::CString;
use std::fs::{File, OpenOptions, Permissions};
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::os::fd::AsRawFd;
use std::os::linux::fs::MetadataExt;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{chroot, PermissionsExt};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use sys_mount::{FilesystemType, Mount, MountFlags};
use tokio::fs;
use walkdir::WalkDir;

use crate::background::GuestBackground;

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

ioctl_write_int_bad!(set_controlling_terminal, TIOCSCTTY);

pub struct GuestInit {}

impl Default for GuestInit {
    fn default() -> Self {
        Self::new()
    }
}

impl GuestInit {
    pub fn new() -> GuestInit {
        GuestInit {}
    }

    pub async fn init(&mut self) -> Result<()> {
        self.early_init().await?;

        trace!("opening console descriptor");
        match OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/console")
        {
            Ok(console) => self.map_console(&console)?,
            Err(error) => warn!("failed to open console: {}", error),
        };

        let idm = IdmClient::open("/dev/hvc1")
            .await
            .map_err(|x| anyhow!("failed to open idm client: {}", x))?;
        self.mount_squashfs_images().await?;

        let config = self.parse_image_config().await?;
        let launch = self.parse_launch_config().await?;

        self.mount_new_root().await?;
        self.nuke_initrd().await?;
        self.bind_new_root().await?;

        if let Some(hostname) = launch.hostname.clone() {
            let result = unsafe {
                sethostname(
                    hostname.as_bytes().as_ptr() as *mut libc::c_char,
                    hostname.len(),
                )
            };
            if result != 0 {
                warn!("failed to set hostname: {}", result);
            }
        }

        if let Some(network) = &launch.network {
            trace!("initializing network");
            if let Err(error) = self.network_setup(network).await {
                warn!("failed to initialize network: {}", error);
            }
        }

        if let Some(cfg) = config.config() {
            trace!("running guest task");
            self.run(cfg, &launch, idm).await?;
        } else {
            return Err(anyhow!(
                "unable to determine what to execute, image config doesn't tell us"
            ));
        }
        Ok(())
    }

    async fn early_init(&mut self) -> Result<()> {
        trace!("early init");
        self.create_dir("/dev", Some(0o0755)).await?;
        self.create_dir("/proc", None).await?;
        self.create_dir("/sys", Some(0o0555)).await?;
        self.create_dir("/root", Some(0o0700)).await?;
        self.create_dir("/tmp", None).await?;
        self.create_dir("/run", Some(0o0755)).await?;
        self.mount_kernel_fs("devtmpfs", "/dev", "mode=0755", None)
            .await?;
        self.mount_kernel_fs("proc", "/proc", "", None).await?;
        self.mount_kernel_fs("sysfs", "/sys", "", None).await?;
        fs::symlink("/proc/self/fd", "/dev/fd").await?;
        fs::symlink("/proc/self/fd/0", "/dev/stdin").await?;
        fs::symlink("/proc/self/fd/1", "/dev/stdout").await?;
        fs::symlink("/proc/self/fd/2", "/dev/stderr").await?;
        self.mount_kernel_fs("cgroup2", "/sys/fs/cgroup", "", Some(MountFlags::RELATIME))
            .await?;
        Ok(())
    }

    async fn create_dir(&mut self, path: &str, mode: Option<u32>) -> Result<()> {
        let path = Path::new(path);
        if !path.is_dir() {
            trace!("creating directory {:?}", path);
            fs::create_dir(path).await?;
        }
        if let Some(mode) = mode {
            let permissions = Permissions::from_mode(mode);
            trace!("setting directory {:?} permissions to {:?}", path, mode);
            fs::set_permissions(path, permissions).await?;
        }
        Ok(())
    }

    async fn mount_kernel_fs(
        &mut self,
        fstype: &str,
        path: &str,
        data: &str,
        flags: Option<MountFlags>,
    ) -> Result<()> {
        trace!("mounting kernel fs {} to {}", fstype, path);
        Mount::builder()
            .fstype(FilesystemType::Manual(fstype))
            .flags(MountFlags::NOEXEC | MountFlags::NOSUID | flags.unwrap_or(MountFlags::empty()))
            .data(data)
            .mount(fstype, path)?;
        Ok(())
    }

    fn map_console(&mut self, console: &File) -> Result<()> {
        trace!("mapping console");
        dup2(console.as_raw_fd(), 0)?;
        dup2(console.as_raw_fd(), 1)?;
        dup2(console.as_raw_fd(), 2)?;
        Ok(())
    }

    async fn mount_squashfs_images(&mut self) -> Result<()> {
        trace!("mounting squashfs images");
        let image_mount_path = Path::new(IMAGE_MOUNT_PATH);
        let config_mount_path = Path::new(CONFIG_MOUNT_PATH);
        self.mount_squashfs(Path::new(IMAGE_BLOCK_DEVICE_PATH), image_mount_path)
            .await?;
        self.mount_squashfs(Path::new(CONFIG_BLOCK_DEVICE_PATH), config_mount_path)
            .await?;
        Ok(())
    }

    async fn mount_squashfs(&mut self, from: &Path, to: &Path) -> Result<()> {
        trace!("mounting squashfs image {:?} to {:?}", from, to);
        if !to.is_dir() {
            fs::create_dir(to).await?;
        }
        Mount::builder()
            .fstype(FilesystemType::Manual("squashfs"))
            .flags(MountFlags::RDONLY)
            .mount(from, to)?;
        Ok(())
    }

    async fn mount_move_subtree(&mut self, from: &Path, to: &Path) -> Result<()> {
        trace!("moving subtree {:?} to {:?}", from, to);
        if !to.is_dir() {
            fs::create_dir(to).await?;
        }
        Mount::builder()
            .fstype(FilesystemType::Manual("none"))
            .flags(MountFlags::MOVE)
            .mount(from, to)?;
        Ok(())
    }

    async fn mount_new_root(&mut self) -> Result<()> {
        trace!("mounting new root");
        self.mount_overlay_tmpfs().await?;
        self.bind_image_to_overlay_tmpfs().await?;
        self.mount_overlay_to_new_root().await?;
        std::env::set_current_dir(NEW_ROOT_PATH)?;
        trace!("mounted new root");
        Ok(())
    }

    async fn mount_overlay_tmpfs(&mut self) -> Result<()> {
        fs::create_dir(OVERLAY_MOUNT_PATH).await?;
        Mount::builder()
            .fstype(FilesystemType::Manual("tmpfs"))
            .mount("tmpfs", OVERLAY_MOUNT_PATH)?;
        fs::create_dir(OVERLAY_UPPER_PATH).await?;
        fs::create_dir(OVERLAY_WORK_PATH).await?;
        Ok(())
    }

    async fn bind_image_to_overlay_tmpfs(&mut self) -> Result<()> {
        fs::create_dir(OVERLAY_IMAGE_BIND_PATH).await?;
        Mount::builder()
            .fstype(FilesystemType::Manual("none"))
            .flags(MountFlags::BIND | MountFlags::RDONLY)
            .mount(IMAGE_MOUNT_PATH, OVERLAY_IMAGE_BIND_PATH)?;
        Ok(())
    }

    async fn mount_overlay_to_new_root(&mut self) -> Result<()> {
        fs::create_dir(NEW_ROOT_PATH).await?;
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

    async fn parse_image_config(&mut self) -> Result<ImageConfiguration> {
        let image_config_path = Path::new(IMAGE_CONFIG_JSON_PATH);
        let content = fs::read_to_string(image_config_path).await?;
        let config = serde_json::from_str(&content)?;
        Ok(config)
    }

    async fn parse_launch_config(&mut self) -> Result<LaunchInfo> {
        trace!("parsing launch config");
        let launch_config = Path::new(LAUNCH_CONFIG_JSON_PATH);
        let content = fs::read_to_string(launch_config).await?;
        Ok(serde_json::from_str(&content)?)
    }

    async fn nuke_initrd(&mut self) -> Result<()> {
        trace!("nuking initrd");
        let initrd_dev = fs::metadata("/").await?.st_dev();
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
                let _ = fs::remove_file(item.path()).await;
                trace!("deleting file {:?}", item.path());
            } else if metadata.is_dir() {
                let _ = fs::remove_dir(item.path()).await;
                trace!("deleting directory {:?}", item.path());
            }
        }
        trace!("nuked initrd");
        Ok(())
    }

    async fn bind_new_root(&mut self) -> Result<()> {
        self.mount_move_subtree(Path::new(SYS_PATH), Path::new(NEW_ROOT_SYS_PATH))
            .await?;
        self.mount_move_subtree(Path::new(PROC_PATH), Path::new(NEW_ROOT_PROC_PATH))
            .await?;
        self.mount_move_subtree(Path::new(DEV_PATH), Path::new(NEW_ROOT_DEV_PATH))
            .await?;
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
        trace!("setting up network for link");

        let etc = PathBuf::from_str("/etc")?;
        if !etc.exists() {
            fs::create_dir(etc).await?;
        }
        let resolv = PathBuf::from_str("/etc/resolv.conf")?;
        let mut lines = vec!["# krata resolver configuration".to_string()];
        for nameserver in &network.resolver.nameservers {
            lines.push(format!("nameserver {}", nameserver));
        }

        let mut conf = lines.join("\n");
        conf.push('\n');
        fs::write(resolv, conf).await?;
        self.network_configure_ethtool(network).await?;
        self.network_configure_link(network).await?;
        Ok(())
    }

    async fn network_configure_link(&mut self, network: &LaunchNetwork) -> Result<()> {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle.link().get().match_name("lo".to_string()).execute();
        let Some(link) = links.try_next().await? else {
            warn!("unable to find link named lo");
            return Ok(());
        };

        handle.link().set(link.header.index).up().execute().await?;

        let ipv4_network: IpNetwork = network.ipv4.address.parse()?;
        let ipv4_gateway: Ipv4Addr = network.ipv4.gateway.parse()?;
        let ipv6_network: IpNetwork = network.ipv6.address.parse()?;
        let ipv6_gateway: Ipv6Addr = network.ipv6.gateway.parse()?;

        let mut links = handle
            .link()
            .get()
            .match_name(network.link.clone())
            .execute();
        let Some(link) = links.try_next().await? else {
            warn!("unable to find link named {}", network.link);
            return Ok(());
        };

        handle
            .address()
            .add(link.header.index, ipv4_network.ip(), ipv4_network.prefix())
            .execute()
            .await?;

        let ipv6_result = handle
            .address()
            .add(link.header.index, ipv6_network.ip(), ipv6_network.prefix())
            .execute()
            .await;

        let ipv6_ready = match ipv6_result {
            Ok(()) => true,
            Err(error) => {
                warn!("unable to setup ipv6 network: {}", error);
                false
            }
        };

        handle.link().set(link.header.index).up().execute().await?;

        handle
            .route()
            .add()
            .v4()
            .destination_prefix(Ipv4Addr::UNSPECIFIED, 0)
            .output_interface(link.header.index)
            .gateway(ipv4_gateway)
            .execute()
            .await?;

        if ipv6_ready {
            let ipv6_gw_result = handle
                .route()
                .add()
                .v6()
                .destination_prefix(Ipv6Addr::UNSPECIFIED, 0)
                .output_interface(link.header.index)
                .gateway(ipv6_gateway)
                .execute()
                .await;

            if let Err(error) = ipv6_gw_result {
                warn!("failed to add ipv6 gateway route: {}", error);
            }
        }
        Ok(())
    }

    async fn network_configure_ethtool(&mut self, network: &LaunchNetwork) -> Result<()> {
        let mut handle = EthtoolHandle::new()?;
        handle.set_gso(&network.link, false)?;
        handle.set_tso(&network.link, false)?;
        Ok(())
    }

    async fn run(&mut self, config: &Config, launch: &LaunchInfo, idm: IdmClient) -> Result<()> {
        let mut cmd = match config.cmd() {
            None => vec![],
            Some(value) => value.clone(),
        };

        if launch.run.is_some() {
            cmd.clone_from(launch.run.as_ref().unwrap());
        }

        if let Some(entrypoint) = config.entrypoint() {
            for item in entrypoint.iter().rev() {
                cmd.insert(0, item.to_string());
            }
        }

        if cmd.is_empty() {
            cmd.push("/bin/sh".to_string());
        }

        let path = cmd.remove(0);

        let mut env = HashMap::new();
        if let Some(config_env) = config.env() {
            env.extend(GuestInit::env_map(config_env));
        }
        env.extend(launch.env.clone());
        env.insert("KRATA_CONTAINER".to_string(), "1".to_string());

        // If we were not provided a terminal definition in our launch manifest, we
        // default to xterm as most terminal emulators support the xterm control codes.
        if !env.contains_key("TERM") {
            env.insert("TERM".to_string(), "xterm".to_string());
        }

        let path = GuestInit::resolve_executable(&env, path.into())?;
        let Some(file_name) = path.file_name() else {
            return Err(anyhow!("cannot get file name of command path"));
        };
        let Some(file_name) = file_name.to_str() else {
            return Err(anyhow!("cannot get file name of command path as str"));
        };
        cmd.insert(0, file_name.to_string());
        let env = GuestInit::env_list(env);

        trace!("running guest command: {}", cmd.join(" "));

        let path = CString::new(path.as_os_str().as_bytes())?;
        let cmd = GuestInit::strings_as_cstrings(cmd)?;
        let env = GuestInit::strings_as_cstrings(env)?;
        let mut working_dir = config
            .working_dir()
            .as_ref()
            .map(|x| x.to_string())
            .unwrap_or("/".to_string());

        if working_dir.is_empty() {
            working_dir = "/".to_string();
        }

        let cgroup = self.init_cgroup().await?;
        self.fork_and_exec(idm, cgroup, working_dir, path, cmd, env)
            .await?;
        Ok(())
    }

    async fn init_cgroup(&self) -> Result<Cgroup> {
        trace!("initializing cgroup");
        let hierarchy = cgroups_rs::hierarchies::auto();
        let cgroup = Cgroup::new(hierarchy, "krata-guest-task")?;
        cgroup.set_cgroup_type("threaded")?;
        trace!("initialized cgroup");
        Ok(cgroup)
    }

    fn strings_as_cstrings(values: Vec<String>) -> Result<Vec<CString>> {
        let mut results: Vec<CString> = vec![];
        for value in values {
            results.push(CString::new(value.as_bytes().to_vec())?);
        }
        Ok(results)
    }

    fn env_map(env: &[String]) -> HashMap<String, String> {
        let mut map = HashMap::<String, String>::new();
        for item in env {
            if let Some((key, value)) = item.split_once('=') {
                map.insert(key.to_string(), value.to_string());
            }
        }
        map
    }

    fn resolve_executable(env: &HashMap<String, String>, path: PathBuf) -> Result<PathBuf> {
        if path.is_absolute() {
            return Ok(path);
        }

        if path.is_file() {
            return Ok(path.absolutize()?.to_path_buf());
        }

        if let Some(path_var) = env.get("PATH") {
            for item in path_var.split(':') {
                let mut exe_path: PathBuf = item.into();
                exe_path.push(&path);
                if exe_path.is_file() {
                    return Ok(exe_path);
                }
            }
        }
        Ok(path)
    }

    fn env_list(env: HashMap<String, String>) -> Vec<String> {
        env.iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect::<Vec<String>>()
    }

    async fn fork_and_exec(
        &mut self,
        idm: IdmClient,
        cgroup: Cgroup,
        working_dir: String,
        path: CString,
        cmd: Vec<CString>,
        env: Vec<CString>,
    ) -> Result<()> {
        match unsafe { fork()? } {
            ForkResult::Parent { child } => self.background(idm, cgroup, child).await,
            ForkResult::Child => self.foreground(cgroup, working_dir, path, cmd, env).await,
        }
    }

    async fn foreground(
        &mut self,
        cgroup: Cgroup,
        working_dir: String,
        path: CString,
        cmd: Vec<CString>,
        env: Vec<CString>,
    ) -> Result<()> {
        GuestInit::set_controlling_terminal()?;
        std::env::set_current_dir(working_dir)?;
        cgroup.add_task(CgroupPid::from(std::process::id() as u64))?;
        execve(&path, &cmd, &env)?;
        Ok(())
    }

    fn set_controlling_terminal() -> Result<()> {
        unsafe {
            setsid();
            set_controlling_terminal(io::stdin().as_raw_fd(), 0)?;
        }
        Ok(())
    }

    async fn background(&mut self, idm: IdmClient, cgroup: Cgroup, executed: Pid) -> Result<()> {
        let mut background = GuestBackground::new(idm, cgroup, executed).await?;
        background.run().await?;
        Ok(())
    }
}
