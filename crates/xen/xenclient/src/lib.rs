pub mod boot;
pub mod elfloader;
pub mod error;
pub mod mem;
pub mod sys;

use crate::boot::{BootDomain, BootSetup};
use crate::elfloader::ElfImageLoader;
use crate::error::{Error, Result};
use boot::BootState;
use indexmap::IndexMap;
use log::{debug, trace, warn};
use pci::{PciBdf, XenPciBackend};
use sys::XEN_PAGE_SHIFT;
use tokio::time::timeout;

use std::path::PathBuf;
use std::str::FromStr;
use std::time::Duration;
use uuid::Uuid;
use xencall::sys::{
    CreateDomain, DOMCTL_DEV_RDM_RELAXED, XEN_DOMCTL_CDF_HAP, XEN_DOMCTL_CDF_HVM_GUEST,
    XEN_DOMCTL_CDF_IOMMU, XEN_X86_EMU_LAPIC,
};
use xencall::XenCall;
use xenstore::{
    XsPermission, XsdClient, XsdInterface, XsdTransaction, XS_PERM_NONE, XS_PERM_READ,
    XS_PERM_READ_WRITE,
};

pub mod pci;
pub mod x86pv;
pub mod x86pvh;

#[derive(Clone)]
pub struct XenClient {
    pub store: XsdClient,
    call: XenCall,
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
    pub backend_domid: u32,
    pub name: String,
    pub max_vcpus: u32,
    pub mem_mb: u64,
    pub kernel: Vec<u8>,
    pub initrd: Vec<u8>,
    pub cmdline: String,
    pub disks: Vec<DomainDisk>,
    pub use_console_backend: Option<String>,
    pub channels: Vec<DomainChannel>,
    pub vifs: Vec<DomainNetworkInterface>,
    pub filesystems: Vec<DomainFilesystem>,
    pub event_channels: Vec<DomainEventChannel>,
    pub pcis: Vec<DomainPciDevice>,
    pub extra_keys: Vec<(String, String)>,
    pub extra_rw_paths: Vec<String>,
}

#[derive(Debug)]
pub struct CreatedChannel {
    pub ring_ref: u64,
    pub evtchn: u32,
}

#[derive(Debug)]
pub struct CreatedDomain {
    pub domid: u32,
    pub channels: Vec<CreatedChannel>,
}

#[allow(clippy::too_many_arguments)]
impl XenClient {
    pub async fn open(current_domid: u32) -> Result<XenClient> {
        let store = XsdClient::open().await?;
        let call = XenCall::open(current_domid)?;
        Ok(XenClient { store, call })
    }

    pub async fn create(&self, config: &DomainConfig) -> Result<CreatedDomain> {
        let mut domain = CreateDomain {
            max_vcpus: config.max_vcpus,
            ..Default::default()
        };
        domain.max_vcpus = config.max_vcpus;

        if cfg!(target_arch = "aarch64") {
            domain.flags = XEN_DOMCTL_CDF_HVM_GUEST | XEN_DOMCTL_CDF_HAP;
        } else {
            domain.flags = XEN_DOMCTL_CDF_HVM_GUEST | XEN_DOMCTL_CDF_HAP | XEN_DOMCTL_CDF_IOMMU;
            domain.arch_domain_config.emulation_flags = XEN_X86_EMU_LAPIC;
        }

        let domid = self.call.create_domain(domain).await?;
        match self.init(domid, &domain, config).await {
            Ok(created) => Ok(created),
            Err(err) => {
                // ignore since destroying a domain is best
                // effort when an error occurs
                let _ = self.destroy(domid).await;
                Err(err)
            }
        }
    }

    async fn init(
        &self,
        domid: u32,
        domain: &CreateDomain,
        config: &DomainConfig,
    ) -> Result<CreatedDomain> {
        trace!(
            "XenClient init domid={} domain={:?} config={:?}",
            domid,
            domain,
            config
        );
        let backend_dom_path = self.store.get_domain_path(0).await?;
        let dom_path = self.store.get_domain_path(domid).await?;
        let uuid_string = Uuid::from_bytes(domain.handle).to_string();
        let vm_path = format!("/vm/{}", uuid_string);

        let ro_perm = &[
            XsPermission {
                id: 0,
                perms: XS_PERM_NONE,
            },
            XsPermission {
                id: domid,
                perms: XS_PERM_READ,
            },
        ];

        let rw_perm = &[XsPermission {
            id: domid,
            perms: XS_PERM_READ_WRITE,
        }];

        let no_perm = &[XsPermission {
            id: 0,
            perms: XS_PERM_NONE,
        }];

        {
            let tx = self.store.transaction().await?;

            tx.rm(dom_path.as_str()).await?;
            tx.mknod(dom_path.as_str(), ro_perm).await?;

            tx.rm(vm_path.as_str()).await?;
            tx.mknod(vm_path.as_str(), ro_perm).await?;

            tx.mknod(vm_path.as_str(), no_perm).await?;
            tx.mknod(format!("{}/device", vm_path).as_str(), no_perm)
                .await?;

            tx.write_string(format!("{}/vm", dom_path).as_str(), &vm_path)
                .await?;

            tx.mknod(format!("{}/cpu", dom_path).as_str(), ro_perm)
                .await?;
            tx.mknod(format!("{}/memory", dom_path).as_str(), ro_perm)
                .await?;

            tx.mknod(format!("{}/control", dom_path).as_str(), ro_perm)
                .await?;

            tx.mknod(format!("{}/control/shutdown", dom_path).as_str(), rw_perm)
                .await?;
            tx.mknod(
                format!("{}/control/feature-poweroff", dom_path).as_str(),
                rw_perm,
            )
            .await?;
            tx.mknod(
                format!("{}/control/feature-reboot", dom_path).as_str(),
                rw_perm,
            )
            .await?;
            tx.mknod(
                format!("{}/control/feature-suspend", dom_path).as_str(),
                rw_perm,
            )
            .await?;
            tx.mknod(format!("{}/control/sysrq", dom_path).as_str(), rw_perm)
                .await?;

            tx.mknod(format!("{}/data", dom_path).as_str(), rw_perm)
                .await?;
            tx.mknod(format!("{}/drivers", dom_path).as_str(), rw_perm)
                .await?;
            tx.mknod(format!("{}/feature", dom_path).as_str(), rw_perm)
                .await?;
            tx.mknod(format!("{}/attr", dom_path).as_str(), rw_perm)
                .await?;
            tx.mknod(format!("{}/error", dom_path).as_str(), rw_perm)
                .await?;

            tx.write_string(
                format!("{}/uuid", vm_path).as_str(),
                &Uuid::from_bytes(domain.handle).to_string(),
            )
            .await?;
            tx.write_string(format!("{}/name", dom_path).as_str(), &config.name)
                .await?;
            tx.write_string(format!("{}/name", vm_path).as_str(), &config.name)
                .await?;

            for (key, value) in &config.extra_keys {
                tx.write_string(format!("{}/{}", dom_path, key).as_str(), value)
                    .await?;
            }

            for path in &config.extra_rw_paths {
                tx.mknod(format!("{}/{}", dom_path, path).as_str(), rw_perm)
                    .await?;
            }

            tx.commit().await?;
        }

        self.call.set_max_vcpus(domid, config.max_vcpus).await?;
        self.call.set_max_mem(domid, config.mem_mb * 1024).await?;
        let xenstore_evtchn: u32;
        let xenstore_mfn: u64;

        let mut domain: BootDomain;
        {
            let loader = ElfImageLoader::load_file_kernel(&config.kernel)?;
            let mut boot =
                BootSetup::new(self.call.clone(), domid, X86PvhPlatform::new(), loader, None);
            domain = boot.initialize(&config.initrd, config.mem_mb).await?;
            boot.boot(&mut domain, &config.cmdline).await?;
            xenstore_evtchn = domain.store_evtchn;
            xenstore_mfn = domain.xenstore_mfn;
        }

        {
            let tx = self.store.transaction().await?;
            tx.write_string(format!("{}/image/os_type", vm_path).as_str(), "linux")
                .await?;
            tx.write_string(
                format!("{}/image/cmdline", vm_path).as_str(),
                &config.cmdline,
            )
            .await?;

            tx.write_string(
                format!("{}/memory/static-max", dom_path).as_str(),
                &(config.mem_mb * 1024).to_string(),
            )
            .await?;
            tx.write_string(
                format!("{}/memory/target", dom_path).as_str(),
                &(config.mem_mb * 1024).to_string(),
            )
            .await?;
            tx.write_string(format!("{}/memory/videoram", dom_path).as_str(), "0")
                .await?;
            tx.write_string(format!("{}/domid", dom_path).as_str(), &domid.to_string())
                .await?;
            tx.write_string(
                format!("{}/store/port", dom_path).as_str(),
                &xenstore_evtchn.to_string(),
            )
            .await?;
            tx.write_string(
                format!("{}/store/ring-ref", dom_path).as_str(),
                &xenstore_mfn.to_string(),
            )
            .await?;
            for i in 0..config.max_vcpus {
                let path = format!("{}/cpu/{}", dom_path, i);
                tx.mkdir(&path).await?;
                tx.set_perms(&path, ro_perm).await?;
                let path = format!("{}/cpu/{}/availability", dom_path, i);
                tx.write_string(&path, "online").await?;
                tx.set_perms(&path, ro_perm).await?;
            }
            tx.commit().await?;
        }
        if !self
            .store
            .introduce_domain(domid, xenstore_mfn, xenstore_evtchn)
            .await?
        {
            return Err(Error::IntroduceDomainFailed);
        }

        let tx = self.store.transaction().await?;
        self.console_device_add(
            &tx,
            &DomainChannel {
                typ: config
                    .use_console_backend
                    .clone()
                    .unwrap_or("xenconsoled".to_string())
                    .to_string(),
                initialized: true,
            },
            &dom_path,
            &backend_dom_path,
            config.backend_domid,
            domid,
            0,
        )
        .await?;

        let mut channels: Vec<CreatedChannel> = Vec::new();
        for (index, channel) in config.channels.iter().enumerate() {
            let (Some(ring_ref), Some(evtchn)) = self
                .console_device_add(
                    &tx,
                    channel,
                    &dom_path,
                    &backend_dom_path,
                    config.backend_domid,
                    domid,
                    index + 1,
                )
                .await?
            else {
                continue;
            };
            channels.push(CreatedChannel { ring_ref, evtchn });
        }

        for (index, disk) in config.disks.iter().enumerate() {
            self.disk_device_add(
                &tx,
                &dom_path,
                &backend_dom_path,
                config.backend_domid,
                domid,
                index,
                disk,
            )
            .await?;
        }

        for (index, filesystem) in config.filesystems.iter().enumerate() {
            self.fs_9p_device_add(
                &tx,
                &dom_path,
                &backend_dom_path,
                config.backend_domid,
                domid,
                index,
                filesystem,
            )
            .await?;
        }

        for (index, vif) in config.vifs.iter().enumerate() {
            self.vif_device_add(
                &tx,
                &dom_path,
                &backend_dom_path,
                config.backend_domid,
                domid,
                index,
                vif,
            )
            .await?;
        }

        for (index, pci) in config.pcis.iter().enumerate() {
            self.pci_device_add(
                &tx,
                &dom_path,
                &backend_dom_path,
                config.backend_domid,
                domid,
                index,
                config.pcis.len(),
                pci,
            )
            .await?;
        }

        for channel in &config.event_channels {
            let id = self
                .call
                .evtchn_alloc_unbound(domid, config.backend_domid)
                .await?;
            let channel_path = format!("{}/evtchn/{}", dom_path, channel.name);
            tx.write_string(&format!("{}/name", channel_path), &channel.name)
                .await?;
            tx.write_string(&format!("{}/channel", channel_path), &id.to_string())
                .await?;
        }

        tx.commit().await?;

        self.call.unpause_domain(domid).await?;
        Ok(CreatedDomain { domid, channels })
    }

    async fn disk_device_add(
        &self,
        tx: &XsdTransaction,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        disk: &DomainDisk,
    ) -> Result<()> {
        let id = (202 << 8) | (index << 4) as u64;
        let backend_items: Vec<(&str, String)> = vec![
            ("frontend-id", domid.to_string()),
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
            ("backend-id", backend_domid.to_string()),
            ("state", "1".to_string()),
            ("virtual-device", id.to_string()),
            ("device-type", "disk".to_string()),
            ("trusted", "1".to_string()),
            ("protocol", "x86_64-abi".to_string()),
        ];

        self.device_add(
            tx,
            "vbd",
            id,
            dom_path,
            backend_dom_path,
            backend_domid,
            domid,
            frontend_items,
            backend_items,
        )
        .await?;
        Ok(())
    }

    #[allow(clippy::unnecessary_unwrap)]
    async fn console_device_add(
        &self,
        tx: &XsdTransaction,
        channel: &DomainChannel,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
    ) -> Result<(Option<u64>, Option<u32>)> {
        let console = domain.consoles.get(index);
        let port = console.map(|x| x.0);
        let ring = console.map(|x| x.1);

        let mut backend_entries = vec![
            ("frontend-id", domid.to_string()),
            ("online", "1".to_string()),
            ("protocol", "vt100".to_string()),
        ];

        let mut frontend_entries = vec![
            ("backend-id", backend_domid.to_string()),
            ("limit", "1048576".to_string()),
            ("output", "pty".to_string()),
            ("tty", "".to_string()),
        ];

        frontend_entries.push(("type", channel.typ.clone()));
        backend_entries.push(("type", channel.typ.clone()));

        if port.is_some() && ring.is_some() {
            if channel.typ != "xenconsoled" {
                frontend_entries.push(("state", "1".to_string()));
            }

            frontend_entries.extend_from_slice(&[
                ("port", port.unwrap().to_string()),
                ("ring-ref", ring.unwrap().to_string()),
            ]);
        } else {
            frontend_entries.extend_from_slice(&[
                ("state", "1".to_string()),
                ("protocol", "vt100".to_string()),
            ]);
        }

        if channel.initialized {
            backend_entries.push(("state", "4".to_string()));
        } else {
            backend_entries.push(("state", "1".to_string()));
        }

        self.device_add(
            tx,
            "console",
            index as u64,
            dom_path,
            backend_dom_path,
            backend_domid,
            domid,
            frontend_entries,
            backend_entries,
        )
        .await?;
        Ok((ring, port))
    }

    async fn fs_9p_device_add(
        &self,
        tx: &XsdTransaction,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        filesystem: &DomainFilesystem,
    ) -> Result<()> {
        let id = 90 + index as u64;
        let backend_items: Vec<(&str, String)> = vec![
            ("frontend-id", domid.to_string()),
            ("online", "1".to_string()),
            ("state", "1".to_string()),
            ("path", filesystem.path.to_string()),
            ("security-model", "none".to_string()),
        ];

        let frontend_items: Vec<(&str, String)> = vec![
            ("backend-id", backend_domid.to_string()),
            ("state", "1".to_string()),
            ("tag", filesystem.tag.to_string()),
        ];

        self.device_add(
            tx,
            "9pfs",
            id,
            dom_path,
            backend_dom_path,
            backend_domid,
            domid,
            frontend_items,
            backend_items,
        )
        .await?;
        Ok(())
    }

    async fn vif_device_add(
        &self,
        tx: &XsdTransaction,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        vif: &DomainNetworkInterface,
    ) -> Result<()> {
        let id = 20 + index as u64;
        let mut backend_items: Vec<(&str, String)> = vec![
            ("frontend-id", domid.to_string()),
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
            ("backend-id", backend_domid.to_string()),
            ("state", "1".to_string()),
            ("mac", vif.mac.to_string()),
            ("trusted", "1".to_string()),
            ("mtu", vif.mtu.to_string()),
        ];

        self.device_add(
            tx,
            "vif",
            id,
            dom_path,
            backend_dom_path,
            backend_domid,
            domid,
            frontend_items,
            backend_items,
        )
        .await?;
        Ok(())
    }

    async fn pci_device_add(
        &self,
        tx: &XsdTransaction,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
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
                self.call
                    .ioport_permission(domid, resource.start as u32, resource.size() as u32, true)
                    .await?;
            } else {
                self.call
                    .iomem_permission(
                        domid,
                        resource.start >> XEN_PAGE_SHIFT,
                        (resource.size() + (XEN_PAGE_SHIFT - 1)) >> XEN_PAGE_SHIFT,
                        true,
                    )
                    .await?;
            }
        }

        if let Some(irq) = backend.read_irq(&device.bdf).await? {
            let irq = self.call.map_pirq(domid, irq as isize, None).await?;
            self.call.irq_permission(domid, irq, true).await?;
        }

        backend.reset(&device.bdf).await?;

        self.call
            .assign_device(
                domid,
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
                ("frontend-id", domid.to_string()),
                ("online", "1".to_string()),
                ("state", "1".to_string()),
                ("num_devs", device_count.to_string()),
            ];

            let frontend_items: Vec<(&str, String)> = vec![
                ("backend-id", backend_domid.to_string()),
                ("state", "1".to_string()),
            ];

            self.device_add(
                tx,
                "pci",
                id,
                dom_path,
                backend_dom_path,
                backend_domid,
                domid,
                frontend_items,
                backend_items,
            )
            .await?;
        }

        let backend_path = format!("{}/backend/{}/{}/{}", backend_dom_path, "pci", domid, id);

        tx.write_string(
            format!("{}/key-{}", backend_path, index),
            &device.bdf.to_string(),
        )
        .await?;
        tx.write_string(
            format!("{}/dev-{}", backend_path, index),
            &device.bdf.to_string(),
        )
        .await?;

        if let Some(vdefn) = device.bdf.vdefn {
            tx.write_string(
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

        tx.write_string(format!("{}/opts-{}", backend_path, index), &options)
            .await?;
        Ok(())
    }

    async fn device_add(
        &self,
        tx: &XsdTransaction,
        typ: &str,
        id: u64,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        frontend_items: Vec<(&str, String)>,
        backend_items: Vec<(&str, String)>,
    ) -> Result<()> {
        let console_zero = typ == "console" && id == 0;

        let frontend_path = if console_zero {
            format!("{}/console", dom_path)
        } else {
            format!("{}/device/{}/{}", dom_path, typ, id)
        };
        let backend_path = format!("{}/backend/{}/{}/{}", backend_dom_path, typ, domid, id);

        let mut backend_items: Vec<(&str, String)> = backend_items.clone();
        let mut frontend_items: Vec<(&str, String)> = frontend_items.clone();
        backend_items.push(("frontend", frontend_path.clone()));
        frontend_items.push(("backend", backend_path.clone()));
        let frontend_perms = &[
            XsPermission {
                id: domid,
                perms: XS_PERM_NONE,
            },
            XsPermission {
                id: backend_domid,
                perms: XS_PERM_READ,
            },
        ];

        let backend_perms = &[
            XsPermission {
                id: backend_domid,
                perms: XS_PERM_NONE,
            },
            XsPermission {
                id: domid,
                perms: XS_PERM_READ,
            },
        ];

        tx.mknod(&frontend_path, frontend_perms).await?;
        for (p, value) in &frontend_items {
            let path = format!("{}/{}", frontend_path, *p);
            tx.write_string(&path, value).await?;
            if !console_zero {
                tx.set_perms(&path, frontend_perms).await?;
            }
        }
        tx.mknod(&backend_path, backend_perms).await?;
        for (p, value) in &backend_items {
            let path = format!("{}/{}", backend_path, *p);
            tx.write_string(&path, value).await?;
        }
        Ok(())
    }

    pub async fn destroy(&self, domid: u32) -> Result<()> {
        if let Err(err) = self.destroy_store(domid).await {
            warn!("failed to destroy store for domain {}: {}", domid, err);
        }
        self.call.destroy_domain(domid).await?;
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
