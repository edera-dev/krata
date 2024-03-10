use std::{
    fs,
    path::{Path, PathBuf},
    str::FromStr,
    sync::Arc,
};

use anyhow::{anyhow, Result};
use ipnetwork::IpNetwork;
use loopdev::LoopControl;
use tokio::sync::Mutex;
use uuid::Uuid;
use xenclient::XenClient;
use xenstore::{XsdClient, XsdInterface};

use self::{
    autoloop::AutoLoop,
    console::XenConsole,
    image::cache::ImageCache,
    launch::{GuestLaunchRequest, GuestLauncher},
};

pub mod autoloop;
pub mod cfgblk;
pub mod console;
pub mod image;
pub mod launch;

pub struct ContainerLoopInfo {
    pub device: String,
    pub file: String,
    pub delete: Option<String>,
}

pub struct GuestState {
    pub exit_code: Option<i32>,
}

pub struct GuestInfo {
    pub uuid: Uuid,
    pub domid: u32,
    pub image: String,
    pub loops: Vec<ContainerLoopInfo>,
    pub ipv4: Option<IpNetwork>,
    pub ipv6: Option<IpNetwork>,
    pub state: GuestState,
}

pub struct RuntimeContext {
    pub image_cache: ImageCache,
    pub autoloop: AutoLoop,
    pub xen: XenClient,
    pub kernel: String,
    pub initrd: String,
}

impl RuntimeContext {
    pub async fn new(store: String) -> Result<Self> {
        let mut image_cache_path = PathBuf::from(&store);
        image_cache_path.push("cache");
        fs::create_dir_all(&image_cache_path)?;

        let xen = XenClient::open(0).await?;
        image_cache_path.push("image");
        fs::create_dir_all(&image_cache_path)?;
        let image_cache = ImageCache::new(&image_cache_path)?;
        let kernel = RuntimeContext::detect_guest_file(&store, "kernel")?;
        let initrd = RuntimeContext::detect_guest_file(&store, "initrd")?;

        Ok(RuntimeContext {
            image_cache,
            autoloop: AutoLoop::new(LoopControl::open()?),
            xen,
            kernel,
            initrd,
        })
    }

    fn detect_guest_file(store: &str, name: &str) -> Result<String> {
        let mut path = PathBuf::from(format!("{}/guest/{}", store, name));
        if path.is_file() {
            return path_as_string(&path);
        }

        path = PathBuf::from(format!("/usr/share/krata/guest/{}", name));
        if path.is_file() {
            return path_as_string(&path);
        }
        Err(anyhow!("unable to find required guest file: {}", name))
    }

    pub async fn list(&mut self) -> Result<Vec<GuestInfo>> {
        let mut guests: Vec<GuestInfo> = Vec::new();
        for domid_candidate in self.xen.store.list("/local/domain").await? {
            let dom_path = format!("/local/domain/{}", domid_candidate);
            let uuid_string = match self
                .xen
                .store
                .read_string(&format!("{}/krata/uuid", &dom_path))
                .await?
            {
                None => continue,
                Some(value) => value,
            };
            let domid =
                u32::from_str(&domid_candidate).map_err(|_| anyhow!("failed to parse domid"))?;
            let uuid = Uuid::from_str(&uuid_string)?;
            let image = self
                .xen
                .store
                .read_string(&format!("{}/krata/image", &dom_path))
                .await?
                .unwrap_or("unknown".to_string());
            let loops = self
                .xen
                .store
                .read_string(&format!("{}/krata/loops", &dom_path))
                .await?;
            let ipv4 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/guest/ipv4", &dom_path))
                .await?;
            let ipv6 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/guest/ipv6", &dom_path))
                .await?;

            let ipv4 = if let Some(ipv4) = ipv4 {
                IpNetwork::from_str(&ipv4).ok()
            } else {
                None
            };

            let ipv6 = if let Some(ipv6) = ipv6 {
                IpNetwork::from_str(&ipv6).ok()
            } else {
                None
            };

            let exit_code = self
                .xen
                .store
                .read_string(&format!("{}/krata/guest/exit-code", &dom_path))
                .await?;

            let exit_code: Option<i32> = match exit_code {
                Some(code) => code.parse().ok(),
                None => None,
            };

            let state = GuestState { exit_code };

            let loops = RuntimeContext::parse_loop_set(&loops);
            guests.push(GuestInfo {
                uuid,
                domid,
                image,
                loops,
                ipv4,
                ipv6,
                state,
            });
        }
        Ok(guests)
    }

    pub async fn resolve(&mut self, id: &str) -> Result<Option<GuestInfo>> {
        for guest in self.list().await? {
            let uuid_string = guest.uuid.to_string();
            let domid_string = guest.domid.to_string();
            if uuid_string == id || domid_string == id || id == format!("krata-{}", uuid_string) {
                return Ok(Some(guest));
            }
        }
        Ok(None)
    }

    fn parse_loop_set(input: &Option<String>) -> Vec<ContainerLoopInfo> {
        let Some(input) = input else {
            return Vec::new();
        };
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

#[derive(Clone)]
pub struct Runtime {
    store: Arc<String>,
    context: Arc<Mutex<RuntimeContext>>,
}

impl Runtime {
    pub async fn new(store: String) -> Result<Self> {
        let context = RuntimeContext::new(store.clone()).await?;
        Ok(Self {
            store: Arc::new(store),
            context: Arc::new(Mutex::new(context)),
        })
    }

    pub async fn launch<'a>(&self, request: GuestLaunchRequest<'a>) -> Result<GuestInfo> {
        let mut context = self.context.lock().await;
        let mut launcher = GuestLauncher::new()?;
        launcher.launch(&mut context, request).await
    }

    pub async fn destroy(&self, id: &str) -> Result<Uuid> {
        let mut context = self.context.lock().await;
        let info = context
            .resolve(id)
            .await?
            .ok_or_else(|| anyhow!("unable to resolve guest: {}", id))?;
        let domid = info.domid;
        let mut store = XsdClient::open().await?;
        let dom_path = store.get_domain_path(domid).await?;
        let uuid = match store
            .read_string(format!("{}/krata/uuid", dom_path).as_str())
            .await?
        {
            None => {
                return Err(anyhow!(
                    "domain {} was not found or not created by krata",
                    domid
                ))
            }
            Some(value) => value,
        };
        if uuid.is_empty() {
            return Err(anyhow!("unable to find krata uuid based on the domain",));
        }
        let uuid = Uuid::parse_str(&uuid)?;
        let loops = store
            .read_string(format!("{}/krata/loops", dom_path).as_str())
            .await?;
        let loops = RuntimeContext::parse_loop_set(&loops);
        context.xen.destroy(domid).await?;
        for info in &loops {
            context.autoloop.unloop(&info.device)?;
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

    pub async fn console(&self, id: &str) -> Result<XenConsole> {
        let mut context = self.context.lock().await;
        let info = context
            .resolve(id)
            .await?
            .ok_or_else(|| anyhow!("unable to resolve guest: {}", id))?;
        let domid = info.domid;
        let tty = context.xen.get_console_path(domid).await?;
        XenConsole::new(&tty).await
    }

    pub async fn list(&self) -> Result<Vec<GuestInfo>> {
        let mut context = self.context.lock().await;
        context.list().await
    }

    pub async fn dupe(&self) -> Result<Runtime> {
        Runtime::new((*self.store).clone()).await
    }
}

fn path_as_string(path: &Path) -> Result<String> {
    path.to_str()
        .ok_or_else(|| anyhow!("unable to convert path to string"))
        .map(|x| x.to_string())
}
