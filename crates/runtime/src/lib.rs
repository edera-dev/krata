use std::{fs, net::Ipv4Addr, path::PathBuf, str::FromStr, sync::Arc};

use anyhow::{anyhow, Result};
use ip::IpVendor;
use ipnetwork::{IpNetwork, Ipv4Network, Ipv6Network};
use krataloopdev::LoopControl;
use log::{debug, error};
use tokio::sync::Semaphore;
use uuid::Uuid;
use xenclient::XenClient;
use xenstore::{XsdClient, XsdInterface};

use self::{
    autoloop::AutoLoop,
    launch::{ZoneLaunchRequest, ZoneLauncher},
    power::PowerManagementContext,
};

pub mod autoloop;
pub mod cfgblk;
pub mod channel;
pub mod ip;
pub mod launch;
pub mod power;

#[cfg(target_arch = "x86_64")]
type RuntimePlatform = xenplatform::x86pv::X86PvPlatform;

#[cfg(not(target_arch = "x86_64"))]
type RuntimePlatform = xenplatform::unsupported::UnsupportedPlatform;

#[derive(Clone)]
pub struct ZoneLoopInfo {
    pub device: String,
    pub file: String,
    pub delete: Option<String>,
}

#[derive(Clone)]
pub struct ZoneState {
    pub exit_code: Option<i32>,
}

#[derive(Clone)]
pub struct ZoneInfo {
    pub name: Option<String>,
    pub uuid: Uuid,
    pub domid: u32,
    pub image: String,
    pub loops: Vec<ZoneLoopInfo>,
    pub zone_ipv4: Option<IpNetwork>,
    pub zone_ipv6: Option<IpNetwork>,
    pub zone_mac: Option<String>,
    pub gateway_ipv4: Option<IpNetwork>,
    pub gateway_ipv6: Option<IpNetwork>,
    pub gateway_mac: Option<String>,
    pub state: ZoneState,
}

#[derive(Clone)]
pub struct RuntimeContext {
    pub autoloop: AutoLoop,
    pub xen: XenClient<RuntimePlatform>,
    pub ipvendor: IpVendor,
}

impl RuntimeContext {
    pub async fn new(host_uuid: Uuid) -> Result<Self> {
        debug!("initializing XenClient");
        let xen = XenClient::new(0, RuntimePlatform::new()).await?;

        debug!("initializing ip allocation vendor");
        let ipv4_network = Ipv4Network::new(Ipv4Addr::new(10, 75, 80, 0), 24)?;
        let ipv6_network = Ipv6Network::from_str("fdd4:1476:6c7e::/48")?;
        let ipvendor =
            IpVendor::new(xen.store.clone(), host_uuid, ipv4_network, ipv6_network).await?;

        debug!("initializing loop devices");
        let autoloop = AutoLoop::new(LoopControl::open()?);

        debug!("krata runtime initialized!");
        Ok(RuntimeContext {
            autoloop,
            xen,
            ipvendor,
        })
    }

    pub async fn list(&self) -> Result<Vec<ZoneInfo>> {
        let mut zones: Vec<ZoneInfo> = Vec::new();
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
            let zone_ipv4 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/zone/ipv4", &dom_path))
                .await?;
            let zone_ipv6 = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/zone/ipv6", &dom_path))
                .await?;
            let zone_mac = self
                .xen
                .store
                .read_string(&format!("{}/krata/network/zone/mac", &dom_path))
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

            let zone_ipv4 = if let Some(zone_ipv4) = zone_ipv4 {
                IpNetwork::from_str(&zone_ipv4).ok()
            } else {
                None
            };

            let zone_ipv6 = if let Some(zone_ipv6) = zone_ipv6 {
                IpNetwork::from_str(&zone_ipv6).ok()
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
                .read_string(&format!("{}/krata/zone/exit-code", &dom_path))
                .await?;

            let exit_code: Option<i32> = match exit_code {
                Some(code) => code.parse().ok(),
                None => None,
            };

            let state = ZoneState { exit_code };

            let loops = RuntimeContext::parse_loop_set(&loops);
            zones.push(ZoneInfo {
                name,
                uuid,
                domid,
                image,
                loops,
                zone_ipv4,
                zone_ipv6,
                zone_mac,
                gateway_ipv4,
                gateway_ipv6,
                gateway_mac,
                state,
            });
        }
        Ok(zones)
    }

    pub async fn resolve(&self, uuid: Uuid) -> Result<Option<ZoneInfo>> {
        for zone in self.list().await? {
            if zone.uuid == uuid {
                return Ok(Some(zone));
            }
        }
        Ok(None)
    }

    fn parse_loop_set(input: &Option<String>) -> Vec<ZoneLoopInfo> {
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
            .map(|(device, file, delete)| ZoneLoopInfo {
                device: device.clone(),
                file: file.clone(),
                delete: if delete == "none" {
                    None
                } else {
                    Some(delete.clone())
                },
            })
            .collect::<Vec<ZoneLoopInfo>>()
    }
}

#[derive(Clone)]
pub struct Runtime {
    host_uuid: Uuid,
    context: RuntimeContext,
    launch_semaphore: Arc<Semaphore>,
}

impl Runtime {
    pub async fn new(host_uuid: Uuid) -> Result<Self> {
        let context = RuntimeContext::new(host_uuid).await?;
        Ok(Self {
            host_uuid,
            context,
            launch_semaphore: Arc::new(Semaphore::new(10)),
        })
    }

    pub async fn launch(&self, request: ZoneLaunchRequest) -> Result<ZoneInfo> {
        let mut launcher = ZoneLauncher::new(self.launch_semaphore.clone())?;
        launcher.launch(&self.context, request).await
    }

    pub async fn destroy(&self, uuid: Uuid) -> Result<Uuid> {
        let info = self
            .context
            .resolve(uuid)
            .await?
            .ok_or_else(|| anyhow!("unable to resolve zone: {}", uuid))?;
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
        let ip = self
            .context
            .ipvendor
            .read_domain_assignment(uuid, domid)
            .await?;
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

        if let Some(ip) = ip {
            if let Err(error) = self.context.ipvendor.recall(&ip).await {
                error!(
                    "failed to recall ip assignment for zone {}: {}",
                    uuid, error
                );
            }
        }

        Ok(uuid)
    }

    pub async fn list(&self) -> Result<Vec<ZoneInfo>> {
        self.context.list().await
    }

    pub async fn dupe(&self) -> Result<Runtime> {
        Runtime::new(self.host_uuid).await
    }

    pub async fn power_management_context(&self) -> Result<PowerManagementContext> {
        let context = RuntimeContext::new(self.host_uuid).await?;
        Ok(PowerManagementContext { context })
    }
}
