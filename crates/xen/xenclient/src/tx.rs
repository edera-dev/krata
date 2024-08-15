use indexmap::IndexMap;
use xencall::{sys::DOMCTL_DEV_RDM_RELAXED, XenCall};
use xenplatform::{
    domain::{BaseDomainConfig, CreatedDomain},
    sys::XEN_PAGE_SHIFT,
};
use xenstore::{
    XsPermission, XsdClient, XsdInterface, XsdTransaction, XS_PERM_NONE, XS_PERM_READ,
    XS_PERM_READ_WRITE,
};

use crate::{
    error::{Error, Result},
    pci::XenPciBackend,
    DomainChannel, DomainDisk, DomainFilesystem, DomainNetworkInterface, DomainPciDevice,
    DomainPciRdmReservePolicy,
};

pub struct ClientTransaction {
    tx: XsdTransaction,
    abort: bool,
    domid: u32,
    dom_path: String,
    backend_domid: u32,
    backend_dom_path: String,
}

impl ClientTransaction {
    pub async fn new(store: &XsdClient, domid: u32, backend_domid: u32) -> Result<Self> {
        let backend_dom_path = store.get_domain_path(0).await?;
        let dom_path = store.get_domain_path(domid).await?;
        Ok(ClientTransaction {
            tx: store.transaction().await?,
            abort: true,
            domid,
            dom_path,
            backend_domid,
            backend_dom_path,
        })
    }

    pub async fn add_domain_declaration(
        &self,
        name: impl AsRef<str>,
        base: &BaseDomainConfig,
        created: &CreatedDomain,
    ) -> Result<()> {
        let vm_path = format!("/vm/{}", base.uuid);
        let ro_perm = &[
            XsPermission {
                id: 0,
                perms: XS_PERM_NONE,
            },
            XsPermission {
                id: self.domid,
                perms: XS_PERM_READ,
            },
        ];

        let no_perm = &[XsPermission {
            id: 0,
            perms: XS_PERM_NONE,
        }];

        let rw_perm = &[XsPermission {
            id: self.domid,
            perms: XS_PERM_READ_WRITE,
        }];

        self.tx.rm(&self.dom_path).await?;
        self.tx.mknod(&self.dom_path, ro_perm).await?;

        self.tx.rm(&vm_path).await?;
        self.tx.mknod(&vm_path, ro_perm).await?;

        self.tx.mknod(&vm_path, no_perm).await?;
        self.tx
            .mknod(format!("{}/device", vm_path).as_str(), no_perm)
            .await?;

        self.tx
            .write_string(format!("{}/vm", self.dom_path).as_str(), &vm_path)
            .await?;

        self.tx
            .mknod(format!("{}/cpu", self.dom_path).as_str(), ro_perm)
            .await?;
        self.tx
            .mknod(format!("{}/memory", self.dom_path).as_str(), ro_perm)
            .await?;

        self.tx
            .mknod(format!("{}/control", self.dom_path).as_str(), ro_perm)
            .await?;

        self.tx
            .mknod(
                format!("{}/control/shutdown", self.dom_path).as_str(),
                rw_perm,
            )
            .await?;
        self.tx
            .mknod(
                format!("{}/control/feature-poweroff", self.dom_path).as_str(),
                rw_perm,
            )
            .await?;
        self.tx
            .mknod(
                format!("{}/control/feature-reboot", self.dom_path).as_str(),
                rw_perm,
            )
            .await?;
        self.tx
            .mknod(
                format!("{}/control/feature-suspend", self.dom_path).as_str(),
                rw_perm,
            )
            .await?;
        self.tx
            .mknod(format!("{}/control/sysrq", self.dom_path).as_str(), rw_perm)
            .await?;

        self.tx
            .mknod(format!("{}/data", self.dom_path).as_str(), rw_perm)
            .await?;
        self.tx
            .mknod(format!("{}/drivers", self.dom_path).as_str(), rw_perm)
            .await?;
        self.tx
            .mknod(format!("{}/feature", self.dom_path).as_str(), rw_perm)
            .await?;
        self.tx
            .mknod(format!("{}/attr", self.dom_path).as_str(), rw_perm)
            .await?;
        self.tx
            .mknod(format!("{}/error", self.dom_path).as_str(), rw_perm)
            .await?;

        self.tx
            .write_string(format!("{}/uuid", vm_path).as_str(), &base.uuid.to_string())
            .await?;
        self.tx
            .write_string(format!("{}/name", self.dom_path).as_str(), name.as_ref())
            .await?;
        self.tx
            .write_string(format!("{}/name", vm_path).as_str(), name.as_ref())
            .await?;

        self.tx
            .write_string(format!("{}/image/os_type", vm_path).as_str(), "linux")
            .await?;
        self.tx
            .write_string(format!("{}/image/cmdline", vm_path).as_str(), &base.cmdline)
            .await?;
        self.tx
            .write_string(
                format!("{}/memory/static-max", self.dom_path).as_str(),
                &(base.max_mem_mb * 1024).to_string(),
            )
            .await?;
        self.tx
            .write_string(
                format!("{}/memory/target", self.dom_path).as_str(),
                &(base.target_mem_mb * 1024).to_string(),
            )
            .await?;
        self.tx
            .write_string(format!("{}/memory/videoram", self.dom_path).as_str(), "0")
            .await?;
        self.tx
            .write_string(
                format!("{}/domid", self.dom_path).as_str(),
                &created.domid.to_string(),
            )
            .await?;
        self.tx
            .write_string(format!("{}/type", self.dom_path).as_str(), "PV")
            .await?;
        self.tx
            .write_string(
                format!("{}/store/port", self.dom_path).as_str(),
                &created.store_evtchn.to_string(),
            )
            .await?;
        self.tx
            .write_string(
                format!("{}/store/ring-ref", self.dom_path).as_str(),
                &created.store_mfn.to_string(),
            )
            .await?;
        for i in 0..base.max_vcpus {
            let path = format!("{}/cpu/{}", self.dom_path, i);
            self.tx.mkdir(&path).await?;
            self.tx.set_perms(&path, ro_perm).await?;
            let path = format!("{}/cpu/{}/availability", self.dom_path, i);
            self.tx
                .write_string(
                    &path,
                    if i < base.target_vcpus {
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

    pub async fn write_key(&self, key: impl AsRef<str>, value: impl AsRef<str>) -> Result<()> {
        self.tx
            .write_string(
                &format!("{}/{}", self.dom_path, key.as_ref()),
                value.as_ref(),
            )
            .await?;
        Ok(())
    }

    pub async fn add_rw_path(&self, key: impl AsRef<str>) -> Result<()> {
        let rw_perm = &[XsPermission {
            id: self.domid,
            perms: XS_PERM_READ_WRITE,
        }];

        self.tx
            .mknod(&format!("{}/{}", self.dom_path, key.as_ref()), rw_perm)
            .await?;
        Ok(())
    }

    pub async fn add_device(
        &self,
        typ: impl AsRef<str>,
        id: u64,
        frontend_items: Vec<(&str, String)>,
        backend_items: Vec<(&str, String)>,
    ) -> Result<()> {
        let console_zero = typ.as_ref() == "console" && id == 0;

        let frontend_path = if console_zero {
            format!("{}/console", self.dom_path)
        } else {
            format!("{}/device/{}/{}", self.dom_path, typ.as_ref(), id)
        };
        let backend_path = format!(
            "{}/backend/{}/{}/{}",
            self.backend_dom_path,
            typ.as_ref(),
            self.domid,
            id
        );

        let mut backend_items: Vec<(&str, String)> = backend_items.clone();
        let mut frontend_items: Vec<(&str, String)> = frontend_items.clone();
        backend_items.push(("frontend", frontend_path.clone()));
        frontend_items.push(("backend", backend_path.clone()));
        let frontend_perms = &[
            XsPermission {
                id: self.domid,
                perms: XS_PERM_NONE,
            },
            XsPermission {
                id: self.backend_domid,
                perms: XS_PERM_READ,
            },
        ];

        let backend_perms = &[
            XsPermission {
                id: self.backend_domid,
                perms: XS_PERM_NONE,
            },
            XsPermission {
                id: self.domid,
                perms: XS_PERM_READ,
            },
        ];

        self.tx.mknod(&frontend_path, frontend_perms).await?;
        for (p, value) in &frontend_items {
            let path = format!("{}/{}", frontend_path, *p);
            self.tx.write_string(&path, value).await?;
            if !console_zero {
                self.tx.set_perms(&path, frontend_perms).await?;
            }
        }
        self.tx.mknod(&backend_path, backend_perms).await?;
        for (p, value) in &backend_items {
            let path = format!("{}/{}", backend_path, *p);
            self.tx.write_string(&path, value).await?;
        }
        Ok(())
    }

    pub async fn add_vbd_device(&self, index: usize, disk: &DomainDisk) -> Result<()> {
        let id = (202 << 8) | (index << 4) as u64;
        let backend_items: Vec<(&str, String)> = vec![
            ("frontend-id", self.domid.to_string()),
            ("online", "1".to_string()),
            ("removable", "0".to_string()),
            ("bootable", "1".to_string()),
            ("state", "1".to_string()),
            ("dev", disk.vdev.to_string()),
            ("type", "phy".to_string()),
            ("mode", if disk.writable { "w" } else { "r" }.to_string()),
            ("device-type", "disk".to_string()),
            ("discard-enable", "0".to_string()),
            ("specification", "xen".to_string()),
            ("physical-device-path", disk.block.path.to_string()),
            (
                "physical-device",
                format!("{:02x}:{:02x}", disk.block.major, disk.block.minor),
            ),
        ];

        let frontend_items: Vec<(&str, String)> = vec![
            ("backend-id", self.backend_domid.to_string()),
            ("state", "1".to_string()),
            ("virtual-device", id.to_string()),
            ("device-type", "disk".to_string()),
            ("trusted", "1".to_string()),
            ("protocol", "x86_64-abi".to_string()),
        ];

        self.add_device("vbd", id, frontend_items, backend_items)
            .await?;
        Ok(())
    }

    pub async fn add_vif_device(&self, index: usize, vif: &DomainNetworkInterface) -> Result<()> {
        let id = 20 + index as u64;
        let mut backend_items: Vec<(&str, String)> = vec![
            ("frontend-id", self.domid.to_string()),
            ("online", "1".to_string()),
            ("state", "1".to_string()),
            ("mac", vif.mac.to_string()),
            ("mtu", vif.mtu.to_string()),
            ("type", "vif".to_string()),
            ("handle", id.to_string()),
        ];

        if vif.bridge.is_some() {
            backend_items.extend_from_slice(&[("bridge", vif.bridge.clone().unwrap())]);
        }

        if vif.script.is_some() {
            backend_items.extend_from_slice(&[
                ("script", vif.script.clone().unwrap()),
                ("hotplug-status", "".to_string()),
            ]);
        } else {
            backend_items.extend_from_slice(&[
                ("script", "".to_string()),
                ("hotplug-status", "connected".to_string()),
            ]);
        }

        let frontend_items: Vec<(&str, String)> = vec![
            ("backend-id", self.backend_domid.to_string()),
            ("state", "1".to_string()),
            ("mac", vif.mac.to_string()),
            ("trusted", "1".to_string()),
            ("mtu", vif.mtu.to_string()),
        ];

        self.add_device("vif", id, frontend_items, backend_items)
            .await?;
        Ok(())
    }

    pub async fn add_9pfs_device(&self, index: usize, filesystem: &DomainFilesystem) -> Result<()> {
        let id = 90 + index as u64;
        let backend_items: Vec<(&str, String)> = vec![
            ("frontend-id", self.domid.to_string()),
            ("online", "1".to_string()),
            ("state", "1".to_string()),
            ("path", filesystem.path.to_string()),
            ("security-model", "none".to_string()),
        ];

        let frontend_items: Vec<(&str, String)> = vec![
            ("backend-id", self.backend_domid.to_string()),
            ("state", "1".to_string()),
            ("tag", filesystem.tag.to_string()),
        ];

        self.add_device("9pfs", id, frontend_items, backend_items)
            .await?;
        Ok(())
    }

    pub async fn add_channel_device(
        &self,
        domain: &CreatedDomain,
        index: usize,
        channel: &DomainChannel,
    ) -> Result<()> {
        let port = domain.console_evtchn;
        let ring = domain.console_mfn;

        let mut backend_items = vec![
            ("frontend-id", self.domid.to_string()),
            ("online", "1".to_string()),
            ("protocol", "vt100".to_string()),
        ];

        let mut frontend_items = vec![
            ("backend-id", self.backend_domid.to_string()),
            ("limit", "1048576".to_string()),
            ("output", "pty".to_string()),
            ("tty", "".to_string()),
        ];

        frontend_items.push(("type", channel.typ.clone()));
        backend_items.push(("type", channel.typ.clone()));

        if index == 0 {
            if channel.typ != "xenconsoled" {
                frontend_items.push(("state", "1".to_string()));
            }

            frontend_items
                .extend_from_slice(&[("port", port.to_string()), ("ring-ref", ring.to_string())]);
        } else {
            frontend_items.extend_from_slice(&[
                ("state", "1".to_string()),
                ("protocol", "vt100".to_string()),
            ]);
        }

        if channel.initialized {
            backend_items.push(("state", "4".to_string()));
        } else {
            backend_items.push(("state", "1".to_string()));
        }

        self.add_device("console", index as u64, frontend_items, backend_items)
            .await?;
        Ok(())
    }

    pub async fn add_pci_device(
        &self,
        call: &XenCall,
        index: usize,
        device_count: usize,
        device: &DomainPciDevice,
    ) -> Result<()> {
        let backend = XenPciBackend::new();
        if !backend.is_assigned(&device.bdf).await? {
            return Err(Error::PciDeviceNotAssignable(device.bdf));
        }
        let resources = backend.read_resources(&device.bdf).await?;
        for resource in resources {
            if resource.is_bar_io() {
                call.ioport_permission(
                    self.domid,
                    resource.start as u32,
                    resource.size() as u32,
                    true,
                )
                .await?;
            } else {
                call.iomem_permission(
                    self.domid,
                    resource.start >> XEN_PAGE_SHIFT,
                    (resource.size() + (XEN_PAGE_SHIFT - 1)) >> XEN_PAGE_SHIFT,
                    true,
                )
                .await?;
            }
        }

        if let Some(irq) = backend.read_irq(&device.bdf).await? {
            let irq = call.map_pirq(self.domid, irq as isize, None).await?;
            call.irq_permission(self.domid, irq, true).await?;
        }

        backend.reset(&device.bdf).await?;

        call.assign_device(
            self.domid,
            device.bdf.encode(),
            if device.rdm_reserve_policy == DomainPciRdmReservePolicy::Relaxed {
                DOMCTL_DEV_RDM_RELAXED
            } else {
                0
            },
        )
        .await?;

        if device.permissive {
            backend.enable_permissive(&device.bdf).await?;
        }

        let id = 60;

        if index == 0 {
            let backend_items: Vec<(&str, String)> = vec![
                ("frontend-id", self.domid.to_string()),
                ("online", "1".to_string()),
                ("state", "1".to_string()),
                ("num_devs", device_count.to_string()),
            ];

            let frontend_items: Vec<(&str, String)> = vec![
                ("backend-id", self.backend_domid.to_string()),
                ("state", "1".to_string()),
            ];

            self.add_device("pci", id, frontend_items, backend_items)
                .await?;
        }

        let backend_path = format!(
            "{}/backend/{}/{}/{}",
            self.backend_dom_path, "pci", self.domid, id
        );

        self.tx
            .write_string(
                format!("{}/key-{}", backend_path, index),
                &device.bdf.to_string(),
            )
            .await?;
        self.tx
            .write_string(
                format!("{}/dev-{}", backend_path, index),
                &device.bdf.to_string(),
            )
            .await?;

        if let Some(vdefn) = device.bdf.vdefn {
            self.tx
                .write_string(
                    format!("{}/vdefn-{}", backend_path, index),
                    &format!("{:#x}", vdefn),
                )
                .await?;
        }

        let mut options = IndexMap::new();
        options.insert("permissive", if device.permissive { "1" } else { "0" });
        options.insert("rdm_policy", device.rdm_reserve_policy.to_option_str());
        options.insert("msitranslate", if device.msi_translate { "1" } else { "0" });
        options.insert(
            "power_mgmt",
            if device.power_management { "1" } else { "0" },
        );
        let options = options
            .into_iter()
            .map(|(key, value)| format!("{}={}", key, value))
            .collect::<Vec<_>>()
            .join(",");

        self.tx
            .write_string(format!("{}/opts-{}", backend_path, index), &options)
            .await?;
        Ok(())
    }

    pub async fn commit(mut self) -> Result<()> {
        self.abort = false;
        self.tx.commit().await?;
        Ok(())
    }
}

impl Drop for ClientTransaction {
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
