pub mod error;

use crate::error::{Error, Result};
use log::{debug, trace};
use pci::PciBdf;
use tokio::time::timeout;
use tx::ClientTransaction;
use xenplatform::boot::BootSetupPlatform;
use xenplatform::domain::{BaseDomainConfig, BaseDomainManager, CreatedDomain};

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use xencall::XenCall;
use xenstore::{XsdClient, XsdInterface};

pub mod pci;
pub mod tx;

#[derive(Clone)]
pub struct XenClient<P: BootSetupPlatform> {
    pub store: XsdClient,
    call: XenCall,
    domain_manager: Arc<BaseDomainManager<P>>,
}

#[derive(Clone, Debug)]
pub struct BlockDeviceRef {
    pub path: String,
    pub major: u32,
    pub minor: u32,
}

#[derive(Clone, Debug)]
pub struct DomainDisk {
    pub vdev: String,
    pub block: BlockDeviceRef,
    pub writable: bool,
}

#[derive(Clone, Debug)]
pub struct DomainFilesystem {
    pub path: String,
    pub tag: String,
}

#[derive(Clone, Debug)]
pub struct DomainNetworkInterface {
    pub mac: String,
    pub mtu: u32,
    pub bridge: Option<String>,
    pub script: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DomainChannel {
    pub typ: String,
    pub initialized: bool,
}

#[derive(Clone, Debug)]
pub struct DomainEventChannel {
    pub name: String,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum DomainPciRdmReservePolicy {
    Invalid,
    #[default]
    Strict,
    Relaxed,
}

impl DomainPciRdmReservePolicy {
    pub fn to_option_str(&self) -> &str {
        match self {
            DomainPciRdmReservePolicy::Invalid => "-1",
            DomainPciRdmReservePolicy::Strict => "0",
            DomainPciRdmReservePolicy::Relaxed => "1",
        }
    }
}

#[derive(Clone, Debug)]
pub struct DomainPciDevice {
    pub bdf: PciBdf,
    pub permissive: bool,
    pub msi_translate: bool,
    pub power_management: bool,
    pub rdm_reserve_policy: DomainPciRdmReservePolicy,
}

#[derive(Clone, Debug)]
pub struct DomainConfig {
    pub base: BaseDomainConfig,
    pub backend_domid: u32,
    pub name: String,
    pub disks: Vec<DomainDisk>,
    pub swap_console_backend: Option<String>,
    pub channels: Vec<DomainChannel>,
    pub vifs: Vec<DomainNetworkInterface>,
    pub filesystems: Vec<DomainFilesystem>,
    pub pcis: Vec<DomainPciDevice>,
    pub extra_keys: Vec<(String, String)>,
    pub extra_rw_paths: Vec<String>,
}

#[derive(Debug)]
pub struct CreatedChannel {
    pub ring_ref: u64,
    pub evtchn: u32,
}

#[allow(clippy::too_many_arguments)]
impl<P: BootSetupPlatform> XenClient<P> {
    pub async fn new(current_domid: u32, platform: P) -> Result<XenClient<P>> {
        let store = XsdClient::open().await?;
        let call: XenCall = XenCall::open(current_domid)?;
        let domain_manager = BaseDomainManager::new(call.clone(), platform).await?;
        Ok(XenClient {
            store,
            call,
            domain_manager: Arc::new(domain_manager),
        })
    }

    pub async fn create(&self, config: &DomainConfig) -> Result<CreatedDomain> {
        let created = self.domain_manager.create(config.base.clone()).await?;
        match self.init(created.domid, config, &created).await {
            Ok(_) => Ok(created),
            Err(err) => {
                // ignore since destroying a domain is best
                // effort when an error occurs
                let _ = self.domain_manager.destroy(created.domid).await;
                Err(err)
            }
        }
    }

    pub async fn transaction(&self, domid: u32, backend_domid: u32) -> Result<ClientTransaction> {
        ClientTransaction::new(&self.store, domid, backend_domid).await
    }

    async fn init(&self, domid: u32, config: &DomainConfig, created: &CreatedDomain) -> Result<()> {
        trace!("xenclient init domid={} domain={:?}", domid, created);
        let transaction = self.transaction(domid, config.backend_domid).await?;
        transaction
            .add_domain_declaration(&config.name, &config.base, created)
            .await?;
        transaction.commit().await?;
        if !self
            .store
            .introduce_domain(domid, created.store_mfn, created.store_evtchn)
            .await?
        {
            return Err(Error::IntroduceDomainFailed);
        }
        let transaction = self.transaction(domid, config.backend_domid).await?;
        transaction
            .add_channel_device(
                created,
                0,
                &DomainChannel {
                    typ: config
                        .swap_console_backend
                        .clone()
                        .unwrap_or("xenconsoled".to_string())
                        .to_string(),
                    initialized: true,
                },
            )
            .await?;

        for (index, channel) in config.channels.iter().enumerate() {
            transaction
                .add_channel_device(created, index + 1, channel)
                .await?;
        }

        for (index, disk) in config.disks.iter().enumerate() {
            transaction.add_vbd_device(index, disk).await?;
        }

        for (index, filesystem) in config.filesystems.iter().enumerate() {
            transaction.add_9pfs_device(index, filesystem).await?;
        }

        for (index, vif) in config.vifs.iter().enumerate() {
            transaction.add_vif_device(index, vif).await?;
        }

        for (index, pci) in config.pcis.iter().enumerate() {
            transaction
                .add_pci_device(&self.call, index, config.pcis.len(), pci)
                .await?;
        }

        for (key, value) in &config.extra_keys {
            transaction.write_key(key, value).await?;
        }

        for key in &config.extra_rw_paths {
            transaction.add_rw_path(key).await?;
        }

        transaction.commit().await?;
        self.call.unpause_domain(domid).await?;
        Ok(())
    }

    pub async fn destroy(&self, domid: u32) -> Result<()> {
        let _ = self.destroy_store(domid).await;
        self.domain_manager.destroy(domid).await?;
        Ok(())
    }

    async fn destroy_store(&self, domid: u32) -> Result<()> {
        let dom_path = self.store.get_domain_path(domid).await?;
        let vm_path = self.store.read_string(&format!("{}/vm", dom_path)).await?;
        if vm_path.is_none() {
            return Err(Error::DomainNonExistent);
        }

        let mut backend_paths: Vec<String> = Vec::new();
        let console_frontend_path = format!("{}/console", dom_path);
        let console_backend_path = self
            .store
            .read_string(format!("{}/backend", console_frontend_path).as_str())
            .await?;

        for device_category in self
            .store
            .list(format!("{}/device", dom_path).as_str())
            .await?
        {
            for device_id in self
                .store
                .list(format!("{}/device/{}", dom_path, device_category).as_str())
                .await?
            {
                let device_path = format!("{}/device/{}/{}", dom_path, device_category, device_id);
                let Some(backend_path) = self
                    .store
                    .read_string(format!("{}/backend", device_path).as_str())
                    .await?
                else {
                    continue;
                };
                backend_paths.push(backend_path);
            }
        }

        for backend in &backend_paths {
            let state_path = format!("{}/state", backend);
            let mut watch = self.store.create_watch(&state_path).await?;
            let online_path = format!("{}/online", backend);
            let tx = self.store.transaction().await?;
            let state = tx.read_string(&state_path).await?.unwrap_or(String::new());
            if state.is_empty() {
                break;
            }
            tx.write_string(&online_path, "0").await?;
            if !state.is_empty() && u32::from_str(&state).unwrap_or(0) != 6 {
                tx.write_string(&state_path, "5").await?;
            }
            self.store.bind_watch(&watch).await?;
            tx.commit().await?;

            let mut count: u32 = 0;
            loop {
                if count >= 3 {
                    debug!("unable to safely destroy backend: {}", backend);
                    break;
                }
                let _ = timeout(Duration::from_secs(1), watch.receiver.recv()).await;
                let state = self
                    .store
                    .read_string(&state_path)
                    .await?
                    .unwrap_or_else(|| "6".to_string());
                let state = i64::from_str(&state).unwrap_or(-1);
                if state == 6 {
                    break;
                }
                count += 1;
            }
        }

        let tx = self.store.transaction().await?;
        let mut backend_removals: Vec<String> = Vec::new();
        backend_removals.extend_from_slice(backend_paths.as_slice());
        if let Some(backend) = console_backend_path {
            backend_removals.push(backend);
        }
        for path in &backend_removals {
            let path = PathBuf::from(path);
            let parent = path.parent().ok_or(Error::PathParentNotFound)?;
            tx.rm(parent.to_str().ok_or(Error::PathStringConversion)?)
                .await?;
        }
        if let Some(vm_path) = vm_path {
            tx.rm(&vm_path).await?;
        }
        tx.rm(&dom_path).await?;
        tx.commit().await?;
        Ok(())
    }
}
