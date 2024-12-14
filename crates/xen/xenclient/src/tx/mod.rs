pub mod channel;
pub mod fs9p;
pub mod pci;
pub mod vbd;
pub mod vif;

use crate::{
    devalloc::DeviceIdAllocator,
    error::{Error, Result},
};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;
use xenplatform::domain::{PlatformDomainConfig, PlatformDomainInfo};
use xenstore::{
    XsPermission, XsdClient, XsdInterface, XsdTransaction, XS_PERM_NONE, XS_PERM_READ,
    XS_PERM_READ_WRITE,
};

pub struct XenTransaction {
    frontend_domid: u32,
    frontend_dom_path: String,
    backend_domid: u32,
    backend_dom_path: String,
    blkalloc: Arc<Mutex<DeviceIdAllocator>>,
    devalloc: Arc<Mutex<DeviceIdAllocator>>,
    tx: XsdTransaction,
    abort: bool,
}

impl XenTransaction {
    pub async fn new(store: &XsdClient, frontend_domid: u32, backend_domid: u32) -> Result<Self> {
        let frontend_dom_path = store.get_domain_path(frontend_domid).await?;
        let backend_dom_path = store.get_domain_path(backend_domid).await?;
        let tx = store.transaction().await?;

        let devalloc = XenTransaction::load_id_allocator(&tx, "devid", &frontend_dom_path).await?;
        let blkalloc = XenTransaction::load_id_allocator(&tx, "blkid", &frontend_dom_path).await?;

        Ok(XenTransaction {
            frontend_domid,
            frontend_dom_path,
            backend_domid,
            backend_dom_path,
            tx,
            devalloc: Arc::new(Mutex::new(devalloc)),
            blkalloc: Arc::new(Mutex::new(blkalloc)),
            abort: true,
        })
    }

    async fn load_id_allocator(
        tx: &XsdTransaction,
        allocator_type: &str,
        frontend_dom_path: &str,
    ) -> Result<DeviceIdAllocator> {
        let state = tx
            .read(format!(
                "{}/{}-alloc-state",
                frontend_dom_path, allocator_type
            ))
            .await?;
        let allocator = state
            .and_then(|state| DeviceIdAllocator::deserialize(&state))
            .unwrap_or_else(DeviceIdAllocator::new);
        Ok(allocator)
    }

    pub async fn assign_next_devid(&self) -> Result<u64> {
        self.devalloc
            .lock()
            .await
            .allocate()
            .ok_or(Error::DevIdExhausted)
            .map(|x| x as u64)
    }

    pub async fn assign_next_blkidx(&self) -> Result<u32> {
        self.blkalloc
            .lock()
            .await
            .allocate()
            .ok_or(Error::DevIdExhausted)
    }

    pub async fn release_devid(&self, devid: u64) -> Result<()> {
        self.devalloc.lock().await.release(devid as u32);
        Ok(())
    }

    pub async fn release_blkid(&self, blkid: u32) -> Result<()> {
        self.blkalloc.lock().await.release(blkid);
        Ok(())
    }

    pub async fn write(
        &self,
        key: impl AsRef<str>,
        value: impl AsRef<str>,
        perms: Option<&[XsPermission]>,
    ) -> Result<()> {
        let path = format!("{}/{}", self.frontend_dom_path, key.as_ref());
        if let Some(perms) = perms {
            self.tx.mknod(&path, perms).await?;
        }

        // empty string is written by mknod, if perms is set we can skip it.
        if perms.is_none() || perms.is_some() && !value.as_ref().is_empty() {
            self.tx.write_string(path, value.as_ref()).await?;
        }
        Ok(())
    }

    pub async fn add_domain_declaration(
        &self,
        name: Option<impl AsRef<str>>,
        platform: &PlatformDomainConfig,
        created: &PlatformDomainInfo,
    ) -> Result<()> {
        let vm_path = format!("/vm/{}", platform.uuid);
        let ro_perm = &[
            XsPermission {
                id: 0,
                perms: XS_PERM_NONE,
            },
            XsPermission {
                id: self.frontend_domid,
                perms: XS_PERM_READ,
            },
        ];

        let no_perm = &[XsPermission {
            id: 0,
            perms: XS_PERM_NONE,
        }];

        let rw_perm = &[XsPermission {
            id: self.frontend_domid,
            perms: XS_PERM_READ_WRITE,
        }];

        self.tx.rm(&self.frontend_dom_path).await?;
        self.tx.mknod(&self.frontend_dom_path, ro_perm).await?;

        self.tx.rm(&vm_path).await?;
        self.tx.mknod(&vm_path, no_perm).await?;
        self.tx
            .write_string(format!("{}/uuid", vm_path), &platform.uuid.to_string())
            .await?;

        self.write("vm", &vm_path, None).await?;
        self.write("cpu", "", Some(ro_perm)).await?;
        self.write("memory", "", Some(ro_perm)).await?;
        self.write("control", "", Some(ro_perm)).await?;
        self.write("control/shutdown", "", Some(rw_perm)).await?;
        self.write("control/feature-poweroff", "", Some(rw_perm))
            .await?;
        self.write("control/feature-reboot", "", Some(rw_perm))
            .await?;
        self.write("control/feature-suspend", "", Some(rw_perm))
            .await?;
        self.write("control/sysrq", "", Some(rw_perm)).await?;
        self.write("data", "", Some(rw_perm)).await?;
        self.write("drivers", "", Some(rw_perm)).await?;
        self.write("feature", "", Some(rw_perm)).await?;
        self.write("attr", "", Some(rw_perm)).await?;
        self.write("error", "", Some(rw_perm)).await?;
        self.write("uuid", platform.uuid.to_string(), Some(ro_perm))
            .await?;
        if let Some(name) = name {
            self.write("name", name.as_ref(), Some(ro_perm)).await?;
        }
        self.write(
            "memory/static-max",
            (platform.resources.max_memory_mb * 1024).to_string(),
            None,
        )
        .await?;
        self.write(
            "memory/target",
            (platform.resources.assigned_memory_mb * 1024).to_string(),
            None,
        )
        .await?;
        self.write("memory/videoram", "0", None).await?;
        self.write("domid", self.frontend_domid.to_string(), None)
            .await?;
        self.write("type", "PV", None).await?;
        self.write("store/port", created.store_evtchn.to_string(), None)
            .await?;
        self.write("store/ring-ref", created.store_mfn.to_string(), None)
            .await?;
        for i in 0..platform.resources.max_vcpus {
            let path = format!("{}/cpu/{}", self.frontend_dom_path, i);
            self.tx.mkdir(&path).await?;
            self.tx.set_perms(&path, ro_perm).await?;
            let path = format!("{}/cpu/{}/availability", self.frontend_dom_path, i);
            self.tx
                .write_string(
                    &path,
                    if i < platform.resources.assigned_vcpus {
                        "online"
                    } else {
                        "offline"
                    },
                )
                .await?;
            self.tx.set_perms(&path, ro_perm).await?;
        }
        Ok(())
    }

    pub async fn add_device(&self, id: u64, device: DeviceDescription) -> Result<()> {
        let frontend_path = if let Some(ref special_frontend_path) = device.special_frontend_path {
            format!("{}/{}", self.frontend_dom_path, special_frontend_path)
        } else {
            format!(
                "{}/device/{}/{}",
                self.frontend_dom_path, device.frontend_type, id
            )
        };
        let backend_path = format!(
            "{}/backend/{}/{}/{}",
            self.backend_dom_path, device.backend_type, self.frontend_domid, id
        );

        let frontend_perms = &[
            XsPermission {
                id: self.frontend_domid,
                perms: XS_PERM_READ_WRITE,
            },
            XsPermission {
                id: self.backend_domid,
                perms: XS_PERM_READ,
            },
        ];

        let backend_perms = &[
            XsPermission {
                id: self.backend_domid,
                perms: XS_PERM_READ_WRITE,
            },
            XsPermission {
                id: self.frontend_domid,
                perms: XS_PERM_READ,
            },
        ];

        self.tx.mknod(&frontend_path, frontend_perms).await?;
        self.tx.mknod(&backend_path, backend_perms).await?;

        for (key, value) in &device.backend_items {
            let path = format!("{}/{}", backend_path, key);
            self.tx.write_string(&path, value).await?;
        }

        self.tx
            .write_string(format!("{}/frontend", backend_path), &frontend_path)
            .await?;
        self.tx
            .write_string(
                format!("{}/frontend-id", backend_path),
                &self.frontend_domid.to_string(),
            )
            .await?;
        for (key, value) in &device.frontend_items {
            let path = format!("{}/{}", frontend_path, key);
            self.tx.write_string(&path, value).await?;
            if device.special_frontend_path.is_none() {
                self.tx.set_perms(&path, frontend_perms).await?;
            }
        }
        self.tx
            .write_string(format!("{}/backend", frontend_path), &backend_path)
            .await?;
        self.tx
            .write_string(
                format!("{}/backend-id", frontend_path),
                &self.backend_domid.to_string(),
            )
            .await?;
        Ok(())
    }

    pub async fn add_rw_path(&self, key: impl AsRef<str>) -> Result<()> {
        let rw_perm = &[XsPermission {
            id: self.frontend_domid,
            perms: XS_PERM_READ_WRITE,
        }];

        self.tx
            .mknod(
                &format!("{}/{}", self.frontend_dom_path, key.as_ref()),
                rw_perm,
            )
            .await?;
        Ok(())
    }

    async fn before_commit(&self) -> Result<()> {
        let devid_allocator_state = self.devalloc.lock().await.serialize();
        let blkid_allocator_state = self.blkalloc.lock().await.serialize();
        self.tx
            .write(
                format!("{}/devid-alloc-state", self.frontend_dom_path),
                devid_allocator_state,
            )
            .await?;
        self.tx
            .write(
                format!("{}/blkid-alloc-state", self.frontend_dom_path),
                blkid_allocator_state,
            )
            .await?;
        Ok(())
    }

    pub async fn maybe_commit(mut self) -> Result<bool> {
        self.abort = false;
        self.before_commit().await?;
        Ok(self.tx.maybe_commit().await?)
    }

    pub async fn commit(mut self) -> Result<()> {
        self.abort = false;
        self.before_commit().await?;
        self.tx.commit().await?;
        Ok(())
    }
}

impl Drop for XenTransaction {
    fn drop(&mut self) {
        if !self.abort {
            return;
        }
        let tx = self.tx.clone();
        tokio::task::spawn(async move {
            let _ = tx.abort().await;
        });
    }
}

pub struct DeviceDescription {
    frontend_type: String,
    backend_type: String,
    special_frontend_path: Option<String>,
    frontend_items: HashMap<String, String>,
    backend_items: HashMap<String, String>,
}

impl DeviceDescription {
    pub fn new(frontend_type: impl AsRef<str>, backend_type: impl AsRef<str>) -> Self {
        Self {
            frontend_type: frontend_type.as_ref().to_string(),
            backend_type: backend_type.as_ref().to_string(),
            special_frontend_path: None,
            frontend_items: HashMap::new(),
            backend_items: HashMap::new(),
        }
    }

    pub fn special_frontend_path(&mut self, path: impl AsRef<str>) -> &mut Self {
        self.special_frontend_path = Some(path.as_ref().to_string());
        self
    }

    pub fn add_frontend_item(&mut self, key: impl AsRef<str>, value: impl ToString) -> &mut Self {
        self.frontend_items
            .insert(key.as_ref().to_string(), value.to_string());
        self
    }

    pub fn add_backend_item(&mut self, key: impl AsRef<str>, value: impl ToString) -> &mut Self {
        self.backend_items
            .insert(key.as_ref().to_string(), value.to_string());
        self
    }

    pub fn add_frontend_bool(&mut self, key: impl AsRef<str>, value: bool) -> &mut Self {
        self.add_frontend_item(key, if value { "1" } else { "0" })
    }

    pub fn add_backend_bool(&mut self, key: impl AsRef<str>, value: bool) -> &mut Self {
        self.add_backend_item(key, if value { "1" } else { "0" })
    }

    pub fn done(self) -> Self {
        self
    }
}

#[derive(Clone, Debug)]
pub struct DeviceResult {
    pub id: u64,
}

#[derive(Clone, Debug)]
pub struct BlockDeviceResult {
    pub id: u64,
    pub idx: u32,
}

#[async_trait::async_trait]
pub trait DeviceConfig {
    type Result;

    async fn add_to_transaction(&self, tx: &XenTransaction) -> Result<Self::Result>;
}

#[derive(Clone, Debug)]
pub struct BlockDeviceRef {
    pub path: String,
    pub major: u32,
    pub minor: u32,
}

impl BlockDeviceRef {
    pub fn new(path: impl AsRef<str>, major: u32, minor: u32) -> Self {
        Self {
            path: path.as_ref().to_string(),
            major,
            minor,
        }
    }
}
