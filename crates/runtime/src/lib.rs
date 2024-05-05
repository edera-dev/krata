use std::{fs, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{anyhow, Result};
use ipnetwork::IpNetwork;
use loopdev::LoopControl;
use tokio::sync::Semaphore;
use uuid::Uuid;
use xenclient::{x86pv::X86PvPlatform, XenClient};
use xenstore::{XsdClient, XsdInterface};

use self::{
    autoloop::AutoLoop,
    launch::{GuestLaunchRequest, GuestLauncher},
};

pub mod autoloop;
pub mod cfgblk;
pub mod channel;
pub mod launch;

pub struct GuestLoopInfo {
    pub device: String,
    pub file: String,
    pub delete: Option<String>,
}

pub struct GuestState {
    pub exit_code: Option<i32>,
}

pub struct GuestInfo {
    pub name: Option<String>,
    pub uuid: Uuid,
    pub domid: u32,
    pub image: String,
    pub loops: Vec<GuestLoopInfo>,
    pub guest_ipv4: Option<IpNetwork>,
    pub guest_ipv6: Option<IpNetwork>,
    pub guest_mac: Option<String>,
    pub gateway_ipv4: Option<IpNetwork>,
    pub gateway_ipv6: Option<IpNetwork>,
    pub gateway_mac: Option<String>,
    pub state: GuestState,
}

#[derive(Clone)]
pub struct RuntimeContext {
    pub autoloop: AutoLoop,
    pub xen: XenClient<X86PvPlatform>,
}

impl RuntimeContext {
    pub async fn new() -> Result<Self> {
        let xen = XenClient::new(0, X86PvPlatform::new()).await?;
        Ok(RuntimeContext {
            autoloop: AutoLoop::new(LoopControl::open()?),
            xen,
        })
    }

    pub async fn list(&self) -> Result<Vec<GuestInfo>> {
        let mut guests: Vec<GuestInfo> = Vec::new();
        for domid_candidate in self.xen.store.list("/local/domain").await? {
            if domid_candidate == "0" {
                continue;
            }
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

            let name = self
                .xen
                .store
                .read_string(&format!("{}/krata/name", &dom_path))
                .await?;

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
            let guest_ipv4 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/guest/ipv4", &dom_path))
                .await?;
            let guest_ipv6 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/guest/ipv6", &dom_path))
                .await?;
            let guest_mac = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/guest/mac", &dom_path))
                .await?;
            let gateway_ipv4 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/gateway/ipv4", &dom_path))
                .await?;
            let gateway_ipv6 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/gateway/ipv6", &dom_path))
                .await?;
            let gateway_mac = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/gateway/mac", &dom_path))
                .await?;

            let guest_ipv4 = if let Some(guest_ipv4) = guest_ipv4 {
                IpNetwork::from_str(&guest_ipv4).ok()
            } else {
                None
            };

            let guest_ipv6 = if let Some(guest_ipv6) = guest_ipv6 {
                IpNetwork::from_str(&guest_ipv6).ok()
            } else {
                None
            };

            let gateway_ipv4 = if let Some(gateway_ipv4) = gateway_ipv4 {
                IpNetwork::from_str(&gateway_ipv4).ok()
            } else {
                None
            };

            let gateway_ipv6 = if let Some(gateway_ipv6) = gateway_ipv6 {
                IpNetwork::from_str(&gateway_ipv6).ok()
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
                name,
                uuid,
                domid,
                image,
                loops,
                guest_ipv4,
                guest_ipv6,
                guest_mac,
                gateway_ipv4,
                gateway_ipv6,
                gateway_mac,
                state,
            });
        }
        Ok(guests)
    }

    pub async fn resolve(&self, uuid: Uuid) -> Result<Option<GuestInfo>> {
        for guest in self.list().await? {
            if guest.uuid == uuid {
                return Ok(Some(guest));
            }
        }
        Ok(None)
    }

    fn parse_loop_set(input: &Option<String>) -> Vec<GuestLoopInfo> {
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
            .map(|(device, file, delete)| GuestLoopInfo {
                device: device.clone(),
                file: file.clone(),
                delete: if delete == "none" {
                    None
                } else {
                    Some(delete.clone())
                },
            })
            .collect::<Vec<GuestLoopInfo>>()
    }
}

#[derive(Clone)]
pub struct Runtime {
    context: RuntimeContext,
    launch_semaphore: Arc<Semaphore>,
}

impl Runtime {
    pub async fn new() -> Result<Self> {
        let context = RuntimeContext::new().await?;
        Ok(Self {
            context,
            launch_semaphore: Arc::new(Semaphore::new(1)),
        })
    }

    pub async fn launch(&self, request: GuestLaunchRequest) -> Result<GuestInfo> {
        let mut launcher = GuestLauncher::new(self.launch_semaphore.clone())?;
        launcher.launch(&self.context, request).await
    }

    pub async fn destroy(&self, uuid: Uuid) -> Result<Uuid> {
        let info = self
            .context
            .resolve(uuid)
            .await?
            .ok_or_else(|| anyhow!("unable to resolve guest: {}", uuid))?;
        let domid = info.domid;
        let store = XsdClient::open().await?;
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
        self.context.xen.destroy(domid).await?;
        for info in &loops {
            self.context.autoloop.unloop(&info.device).await?;
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

    pub async fn list(&self) -> Result<Vec<GuestInfo>> {
        self.context.list().await
    }

    pub async fn dupe(&self) -> Result<Runtime> {
        Runtime::new().await
    }
}
