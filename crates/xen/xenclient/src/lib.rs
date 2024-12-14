pub mod error;

use config::{DomainConfig, DomainResult};
use error::{Error, Result};
use log::{debug, trace};
use tokio::time::timeout;
use tx::{DeviceConfig, XenTransaction};
use xenplatform::domain::{PlatformDomainInfo, PlatformDomainManager};

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;
use xencall::XenCall;
use xenstore::{XsdClient, XsdInterface};

pub mod config;
pub mod devalloc;
pub mod devstate;
pub mod pci;
pub mod tx;
pub mod util;

#[derive(Clone)]
pub struct XenClient {
    pub store: XsdClient,
    pub call: XenCall,
    domain_manager: Arc<PlatformDomainManager>,
}

#[allow(clippy::too_many_arguments)]
impl XenClient {
    pub async fn new() -> Result<XenClient> {
        let store = XsdClient::open().await?;
        let call: XenCall = XenCall::open(0)?;
        let domain_manager = PlatformDomainManager::new(call.clone()).await?;
        Ok(XenClient {
            store,
            call,
            domain_manager: Arc::new(domain_manager),
        })
    }

    pub async fn create(&self, config: DomainConfig) -> Result<DomainResult> {
        let platform = config
            .get_platform()
            .as_ref()
            .ok_or_else(|| Error::ParameterMissing("platform"))?
            .clone();
        let platform = self.domain_manager.create(platform).await?;
        match self.init(platform.domid, config, &platform).await {
            Ok(result) => Ok(result),
            Err(err) => {
                // ignore since destroying a domain is best-effort when an error occurs
                let _ = self.domain_manager.destroy(platform.domid).await;
                Err(err)
            }
        }
    }

    pub async fn transaction(&self, domid: u32, backend_domid: u32) -> Result<XenTransaction> {
        XenTransaction::new(&self.store, domid, backend_domid).await
    }

    async fn init(
        &self,
        domid: u32,
        mut config: DomainConfig,
        created: &PlatformDomainInfo,
    ) -> Result<DomainResult> {
        trace!("xenclient init domid={} domain={:?}", domid, created);
        let platform_config = config
            .get_platform()
            .as_ref()
            .ok_or_else(|| Error::ParameterMissing("platform"))?;
        loop {
            let transaction = self.transaction(domid, config.get_backend_domid()).await?;
            transaction
                .add_domain_declaration(config.get_name().clone(), platform_config, created)
                .await?;
            if transaction.maybe_commit().await? {
                break;
            }
        }
        if !self
            .store
            .introduce_domain(domid, created.store_mfn, created.store_evtchn)
            .await?
        {
            return Err(Error::IntroduceDomainFailed);
        }
        config.prepare(domid, &self.call, created).await?;
        let mut channels;
        let mut vifs;
        let mut vbds;
        let mut fs9ps;
        let mut pci_result;
        loop {
            let transaction = self.transaction(domid, config.get_backend_domid()).await?;

            channels = Vec::new();
            for channel in config.get_channels() {
                let result = channel.add_to_transaction(&transaction).await?;
                channels.push(result);
            }

            vifs = Vec::new();
            for vif in config.get_vifs() {
                let result = vif.add_to_transaction(&transaction).await?;
                vifs.push(result);
            }

            vbds = Vec::new();
            for vbd in config.get_vbds() {
                let result = vbd.add_to_transaction(&transaction).await?;
                vbds.push(result);
            }

            fs9ps = Vec::new();
            for fs9p in config.get_fs9ps() {
                let result = fs9p.add_to_transaction(&transaction).await?;
                fs9ps.push(result);
            }

            pci_result = None;
            if let Some(pci) = config.get_pci().as_ref() {
                pci_result = Some(pci.add_to_transaction(&transaction).await?);
            }

            for (key, value) in config.get_extra_keys() {
                transaction.write(key, value, None).await?;
            }

            for rw_path in config.get_rw_paths() {
                transaction.add_rw_path(rw_path).await?;
            }

            if transaction.maybe_commit().await? {
                break;
            }
        }

        if config.get_start() {
            self.call.unpause_domain(domid).await?;
        }

        Ok(DomainResult {
            platform: created.clone(),
            channels,
            vifs,
            vbds,
            fs9ps,
            pci: pci_result,
        })
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
            self.destroy_backend(backend).await?;
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

    async fn destroy_backend(&self, backend: &str) -> Result<()> {
        let state_path = format!("{}/state", backend);
        let mut watch = self.store.create_watch(&state_path).await?;
        let online_path = format!("{}/online", backend);
        let tx = self.store.transaction().await?;
        let state = tx.read_string(&state_path).await?.unwrap_or(String::new());
        if state.is_empty() {
            return Ok(());
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
        self.store.rm(backend).await?;
        Ok(())
    }

    pub async fn destroy_device(
        &self,
        category: &str,
        domid: u32,
        devid: u64,
        blkid: Option<u32>,
    ) -> Result<()> {
        let dom_path = self.store.get_domain_path(domid).await?;
        let device_path = format!("{}/device/{}/{}", dom_path, category, devid);
        if let Some(backend_path) = self
            .store
            .read_string(format!("{}/backend", device_path).as_str())
            .await?
        {
            self.destroy_backend(&backend_path).await?;
        }
        self.destroy_backend(&device_path).await?;
        loop {
            let tx = self.transaction(domid, 0).await?;
            tx.release_devid(devid).await?;
            if let Some(blkid) = blkid {
                tx.release_blkid(blkid).await?;
            }
            if tx.maybe_commit().await? {
                break;
            }
        }
        Ok(())
    }
}
