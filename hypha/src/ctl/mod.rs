pub mod cfgblk;

use crate::autoloop::AutoLoop;
use crate::ctl::cfgblk::ConfigBlock;
use crate::error::{HyphaError, Result};
use crate::image::cache::ImageCache;
use crate::image::name::ImageName;
use crate::image::{ImageCompiler, ImageInfo};
use loopdev::LoopControl;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::exit;
use std::str::FromStr;
use std::{fs, io, thread};
use termion::raw::IntoRawMode;
use uuid::Uuid;
use xenclient::{DomainConfig, DomainDisk, XenClient};
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

    pub fn launch(
        &mut self,
        kernel_path: &str,
        initrd_path: &str,
        image: &str,
        vcpus: u32,
        mem: u64,
    ) -> Result<u32> {
        let uuid = Uuid::new_v4();
        let name = format!("hypha-{uuid}");
        let image_info = self.compile(image)?;
        let cfgblk = ConfigBlock::new(&uuid, &image_info)?;
        cfgblk.build()?;

        let image_squashfs_path = image_info
            .image_squashfs
            .to_str()
            .ok_or_else(|| HyphaError::new("failed to convert image squashfs path to string"))?;

        let cfgblk_dir_path = cfgblk
            .dir
            .to_str()
            .ok_or_else(|| HyphaError::new("failed to convert cfgblk directory path to string"))?;
        let cfgblk_squashfs_path = cfgblk
            .file
            .to_str()
            .ok_or_else(|| HyphaError::new("failed to convert cfgblk squashfs path to string"))?;

        let image_squashfs_loop = self.autoloop.loopify(image_squashfs_path)?;
        let cfgblk_squashfs_loop = self.autoloop.loopify(cfgblk_squashfs_path)?;

        let config = DomainConfig {
            backend_domid: 0,
            name: &name,
            max_vcpus: vcpus,
            mem_mb: mem,
            kernel_path,
            initrd_path,
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
                        "{}:{}:none,{}:{}:{}",
                        &image_squashfs_loop.path,
                        image_squashfs_path,
                        &cfgblk_squashfs_loop.path,
                        cfgblk_squashfs_path,
                        cfgblk_dir_path,
                    ),
                ),
                ("hypha/image".to_string(), image.to_string()),
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

    pub fn console(&mut self, domid: u32) -> Result<()> {
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
            let domid = u32::from_str(&domid_candidate)
                .map_err(|_| HyphaError::new("failed to parse domid"))?;
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
            let loops = Controller::parse_loop_set(&loops);
            containers.push(ContainerInfo {
                uuid,
                domid,
                image,
                loops,
            });
        }
        Ok(containers)
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
}
