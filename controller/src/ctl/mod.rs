pub mod cfgblk;

use crate::autoloop::AutoLoop;
use crate::ctl::cfgblk::ConfigBlock;
use crate::image::cache::ImageCache;
use crate::image::name::ImageName;
use crate::image::{ImageCompiler, ImageInfo};
use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use hypha::{
    LaunchInfo, LaunchNetwork, LaunchNetworkIpv4, LaunchNetworkIpv6, LaunchNetworkResolver,
};
use ipnetwork::Ipv4Network;
use loopdev::LoopControl;
use std::io::{Read, Write};
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::exit;
use std::str::FromStr;
use std::{fs, io, thread};
use termion::raw::IntoRawMode;
use uuid::Uuid;
use xenclient::{DomainConfig, DomainDisk, DomainNetworkInterface, XenClient};
use xenstore::client::{XsdClient, XsdInterface};

pub struct Controller {
    image_cache: ImageCache,
    autoloop: AutoLoop,
    client: XenClient,
}

pub struct ContainerLoopInfo {
    pub device: String,
    pub file: String,
    pub delete: Option<String>,
}

pub struct ContainerInfo {
    pub uuid: Uuid,
    pub domid: u32,
    pub image: String,
    pub loops: Vec<ContainerLoopInfo>,
    pub ipv4: String,
}

impl Controller {
    pub fn new(store_path: String) -> Result<Controller> {
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
        })
    }

    fn compile(&mut self, image: &str) -> Result<ImageInfo> {
        let image = ImageName::parse(image)?;
        let compiler = ImageCompiler::new(&self.image_cache)?;
        compiler.compile(&image)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn launch(
        &mut self,
        kernel_path: &str,
        initrd_path: &str,
        config_bundle_path: Option<&str>,
        image: &str,
        vcpus: u32,
        mem: u64,
        env: Option<Vec<String>>,
        run: Option<Vec<String>>,
        debug: bool,
    ) -> Result<(Uuid, u32)> {
        let uuid = Uuid::new_v4();
        let name = format!("hypha-{uuid}");
        let image_info = self.compile(image)?;

        let mut mac = MacAddr6::random();
        mac.set_local(true);
        let ipv4 = self.allocate_ipv4()?;
        let ipv6 = mac.to_link_local_ipv6();
        let launch_config = LaunchInfo {
            network: Some(LaunchNetwork {
                link: "eth0".to_string(),
                ipv4: LaunchNetworkIpv4 {
                    address: format!("{}/24", ipv4),
                    gateway: "192.168.42.1".to_string(),
                },
                ipv6: LaunchNetworkIpv6 {
                    address: format!("{}/10", ipv6),
                    gateway: "fe80::1".to_string(),
                },
                resolver: LaunchNetworkResolver {
                    nameservers: vec![
                        "1.1.1.1".to_string(),
                        "1.0.0.1".to_string(),
                        "2606:4700:4700::1111".to_string(),
                        "2606:4700:4700::1001".to_string(),
                    ],
                },
            }),
            env,
            run,
        };

        let cfgblk = ConfigBlock::new(&uuid, &image_info, config_bundle_path)?;
        cfgblk.build(&launch_config)?;

        let image_squashfs_path = image_info
            .image_squashfs
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert image squashfs path to string"))?;

        let cfgblk_dir_path = cfgblk
            .dir
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk directory path to string"))?;
        let cfgblk_squashfs_path = cfgblk
            .file
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk squashfs path to string"))?;

        let image_squashfs_loop = self.autoloop.loopify(image_squashfs_path)?;
        let cfgblk_squashfs_loop = self.autoloop.loopify(cfgblk_squashfs_path)?;

        let cmdline_options = [if debug { "debug" } else { "quiet" }, "elevator=noop"];
        let cmdline = cmdline_options.join(" ");

        let mac = mac.to_string().replace('-', ":");
        let config = DomainConfig {
            backend_domid: 0,
            name: &name,
            max_vcpus: vcpus,
            mem_mb: mem,
            kernel_path,
            initrd_path,
            cmdline: &cmdline,
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
            consoles: vec![],
            vifs: vec![DomainNetworkInterface {
                mac: &mac,
                mtu: 1500,
                bridge: None,
                script: None,
            }],
            filesystems: vec![],
            extra_keys: vec![
                ("hypha/uuid".to_string(), uuid.to_string()),
                (
                    "hypha/loops".to_string(),
                    format!(
                        "{}:{}:none,{}:{}:{}",
                        &image_squashfs_loop.path,
                        image_squashfs_path,
                        &cfgblk_squashfs_loop.path,
                        cfgblk_squashfs_path,
                        cfgblk_dir_path,
                    ),
                ),
                ("hypha/image".to_string(), image.to_string()),
                ("hypha/ipv4".to_string(), ipv4.to_string()),
            ],
        };
        match self.client.create(&config) {
            Ok(domid) => Ok((uuid, domid)),
            Err(error) => {
                let _ = self.autoloop.unloop(&image_squashfs_loop.path);
                let _ = self.autoloop.unloop(&cfgblk_squashfs_loop.path);
                let _ = fs::remove_dir(&cfgblk.dir);
                Err(error.into())
            }
        }
    }

    pub fn destroy(&mut self, id: &str) -> Result<Uuid> {
        let info = self
            .resolve(id)?
            .ok_or_else(|| anyhow!("unable to resolve container: {}", id))?;
        let domid = info.domid;
        let mut store = XsdClient::open()?;
        let dom_path = store.get_domain_path(domid)?;
        let uuid = match store.read_string_optional(format!("{}/hypha/uuid", dom_path).as_str())? {
            None => {
                return Err(anyhow!(
                    "domain {} was not found or not created by hypha",
                    domid
                ))
            }
            Some(value) => value,
        };
        if uuid.is_empty() {
            return Err(anyhow!("unable to find hypha uuid based on the domain",));
        }
        let uuid = Uuid::parse_str(&uuid)?;
        let loops = store.read_string(format!("{}/hypha/loops", dom_path).as_str())?;
        let loops = Controller::parse_loop_set(&loops);
        self.client.destroy(domid)?;
        for info in &loops {
            self.autoloop.unloop(&info.device)?;
            match &info.delete {
                None => {}
                Some(delete) => {
                    let delete_path = PathBuf::from(delete);
                    if delete_path.is_file() || delete_path.is_symlink() {
                        fs::remove_file(&delete_path)?;
                    } else if delete_path.is_dir() {
                        fs::remove_dir_all(&delete_path)?;
                    }
                }
            }
        }
        Ok(uuid)
    }

    pub fn console(&mut self, id: &str) -> Result<()> {
        let info = self
            .resolve(id)?
            .ok_or_else(|| anyhow!("unable to resolve container: {}", id))?;
        let domid = info.domid;
        let (mut read, mut write) = self.client.open_console(domid)?;
        let mut stdin = io::stdin();
        let is_tty = termion::is_tty(&stdin);
        let mut stdout_for_exit = io::stdout().into_raw_mode()?;
        thread::spawn(move || {
            let mut buffer = vec![0u8; 60];
            loop {
                let size = stdin.read(&mut buffer).expect("failed to read stdin");
                if is_tty && size == 1 && buffer[0] == 0x1d {
                    stdout_for_exit
                        .suspend_raw_mode()
                        .expect("failed to disable raw mode");
                    stdout_for_exit.flush().expect("failed to flush stdout");
                    exit(0);
                }
                write
                    .write_all(&buffer[0..size])
                    .expect("failed to write to domain console");
                write.flush().expect("failed to flush domain console");
            }
        });

        let mut buffer = vec![0u8; 256];
        if is_tty {
            let mut stdout = io::stdout().into_raw_mode()?;
            loop {
                let size = read.read(&mut buffer)?;
                stdout.write_all(&buffer[0..size])?;
                stdout.flush()?;
            }
        } else {
            let mut stdout = io::stdout();
            loop {
                let size = read.read(&mut buffer)?;
                stdout.write_all(&buffer[0..size])?;
                stdout.flush()?;
            }
        }
    }

    pub fn list(&mut self) -> Result<Vec<ContainerInfo>> {
        let mut containers: Vec<ContainerInfo> = Vec::new();
        for domid_candidate in self.client.store.list_any("/local/domain")? {
            let dom_path = format!("/local/domain/{}", domid_candidate);
            let uuid_string = match self
                .client
                .store
                .read_string_optional(&format!("{}/hypha/uuid", &dom_path))?
            {
                None => continue,
                Some(value) => value,
            };
            let domid =
                u32::from_str(&domid_candidate).map_err(|_| anyhow!("failed to parse domid"))?;
            let uuid = Uuid::from_str(&uuid_string)?;
            let image = self
                .client
                .store
                .read_string_optional(&format!("{}/hypha/image", &dom_path))?
                .unwrap_or("unknown".to_string());
            let loops = self
                .client
                .store
                .read_string_optional(&format!("{}/hypha/loops", &dom_path))?
                .unwrap_or("".to_string());
            let ipv4 = self
                .client
                .store
                .read_string_optional(&format!("{}/hypha/ipv4", &dom_path))?
                .unwrap_or("unknown".to_string());
            let loops = Controller::parse_loop_set(&loops);
            containers.push(ContainerInfo {
                uuid,
                domid,
                image,
                loops,
                ipv4,
            });
        }
        Ok(containers)
    }

    pub fn resolve(&mut self, id: &str) -> Result<Option<ContainerInfo>> {
        for container in self.list()? {
            let uuid_string = container.uuid.to_string();
            let domid_string = container.domid.to_string();
            if uuid_string == id || domid_string == id || id == format!("hypha-{}", uuid_string) {
                return Ok(Some(container));
            }
        }
        Ok(None)
    }

    fn parse_loop_set(input: &str) -> Vec<ContainerLoopInfo> {
        let sets = input
            .split(',')
            .map(|x| x.to_string())
            .map(|x| x.split(':').map(|v| v.to_string()).collect::<Vec<String>>())
            .map(|x| (x[0].clone(), x[1].clone(), x[2].clone()))
            .collect::<Vec<(String, String, String)>>();
        sets.iter()
            .map(|(device, file, delete)| ContainerLoopInfo {
                device: device.clone(),
                file: file.clone(),
                delete: if delete == "none" {
                    None
                } else {
                    Some(delete.clone())
                },
            })
            .collect::<Vec<ContainerLoopInfo>>()
    }

    fn allocate_ipv4(&mut self) -> Result<Ipv4Addr> {
        let network = Ipv4Network::new(Ipv4Addr::new(192, 168, 42, 0), 24)?;
        let mut used: Vec<Ipv4Addr> = vec![
            Ipv4Addr::new(192, 168, 42, 0),
            Ipv4Addr::new(192, 168, 42, 1),
            Ipv4Addr::new(192, 168, 42, 255),
        ];
        for domid_candidate in self.client.store.list_any("/local/domain")? {
            let dom_path = format!("/local/domain/{}", domid_candidate);
            let ip_path = format!("{}/hypha/ipv4", dom_path);
            let existing_ip = self.client.store.read_string_optional(&ip_path)?;
            if let Some(existing_ip) = existing_ip {
                used.push(Ipv4Addr::from_str(&existing_ip)?);
            }
        }

        let mut found: Option<Ipv4Addr> = None;
        for ip in network.iter() {
            if !used.contains(&ip) {
                found = Some(ip);
                break;
            }
        }

        if found.is_none() {
            return Err(anyhow!(
                "unable to find ipv4 to allocate to container, ipv4 addresses are exhausted"
            ));
        }

        Ok(found.unwrap())
    }
}
