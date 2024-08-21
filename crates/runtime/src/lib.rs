use anyhow::{anyhow, Result};
use krataloopdev::LoopControl;
use log::debug;
use std::{fs, path::PathBuf, str::FromStr, sync::Arc};
use tokio::sync::Semaphore;
use uuid::Uuid;

use xenclient::XenClient;
use xenplatform::domain::XEN_EXTRA_MEMORY_KB;
use xenstore::{XsdClient, XsdInterface};

use self::{
    autoloop::AutoLoop,
    launch::{ZoneLaunchRequest, ZoneLauncher},
    power::PowerManagementContext,
};

pub mod autoloop;
pub mod cfgblk;
pub mod channel;
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
    pub state: ZoneState,
}

#[derive(Clone)]
pub struct RuntimeContext {
    pub autoloop: AutoLoop,
    pub xen: XenClient<RuntimePlatform>,
}

impl RuntimeContext {
    pub async fn new() -> Result<Self> {
        let xen = XenClient::new(0, RuntimePlatform::new()).await?;
        Ok(RuntimeContext {
            autoloop: AutoLoop::new(LoopControl::open()?),
            xen,
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
    context: RuntimeContext,
    launch_semaphore: Arc<Semaphore>,
}

impl Runtime {
    pub async fn new() -> Result<Self> {
        let context = RuntimeContext::new().await?;
        debug!("testing for hypervisor presence");
        context
            .xen
            .call
            .get_version_capabilities()
            .await
            .map_err(|_| anyhow!("hypervisor is not present"))?;
        Ok(Self {
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

    pub async fn set_memory_resources(
        &self,
        domid: u32,
        target_memory_bytes: u64,
        max_memory_bytes: u64,
    ) -> Result<()> {
        let mut max_memory_bytes = max_memory_bytes + (XEN_EXTRA_MEMORY_KB * 1024);
        if target_memory_bytes > max_memory_bytes {
            max_memory_bytes = target_memory_bytes + (XEN_EXTRA_MEMORY_KB * 1024);
        }

        self.context
            .xen
            .call
            .set_max_mem(domid, max_memory_bytes / 1024)
            .await?;
        let domain_path = self.context.xen.store.get_domain_path(domid).await?;
        let tx = self.context.xen.store.transaction().await?;
        let max_memory_path = format!("{}/memory/static-max", domain_path);
        tx.write_string(max_memory_path, &(max_memory_bytes / 1024).to_string())
            .await?;
        let target_memory_path = format!("{}/memory/target", domain_path);
        tx.write_string(
            target_memory_path,
            &(target_memory_bytes / 1024).to_string(),
        )
        .await?;
        tx.commit().await?;
        Ok(())
    }

    pub async fn set_cpu_resources(&self, domid: u32, target_cpus: u32) -> Result<()> {
        let domain_path = self.context.xen.store.get_domain_path(domid).await?;
        let cpus = self
            .context
            .xen
            .store
            .list(&format!("{}/cpu", domain_path))
            .await?;
        let tx = self.context.xen.store.transaction().await?;
        for cpu in cpus {
            let Some(id) = cpu.parse::<u32>().ok() else {
                continue;
            };
            let available = if id >= target_cpus {
                "offline"
            } else {
                "online"
            };
            tx.write_string(
                format!("{}/cpu/{}/availability", domain_path, id),
                available,
            )
            .await?;
        }
        tx.commit().await?;
        Ok(())
    }

    pub async fn list(&self) -> Result<Vec<ZoneInfo>> {
        self.context.list().await
    }

    pub async fn dupe(&self) -> Result<Runtime> {
        Runtime::new().await
    }

    pub async fn power_management_context(&self) -> Result<PowerManagementContext> {
        let context = RuntimeContext::new().await?;
        Ok(PowerManagementContext { context })
    }

    pub async fn read_hypervisor_console(&self, clear: bool) -> Result<Arc<str>> {
        let index = 0_u32;
        let (rawbuf, newindex) = self
            .context
            .xen
            .call
            .read_console_ring_raw(clear, index)
            .await?;
        let buf = std::str::from_utf8(&rawbuf[..newindex as usize])?;
        Ok(Arc::from(buf))
    }
}
