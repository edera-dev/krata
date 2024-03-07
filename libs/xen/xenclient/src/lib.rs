pub mod boot;
pub mod elfloader;
pub mod error;
pub mod mem;
pub mod sys;
pub mod x86;

use crate::boot::BootSetup;
use crate::elfloader::ElfImageLoader;
use crate::error::{Error, Result};
use crate::x86::X86BootSetup;
use log::{trace, warn};

use std::fs::read;
use std::path::PathBuf;
use std::str::FromStr;
use std::thread;
use std::time::Duration;
use uuid::Uuid;
use xencall::sys::CreateDomain;
use xencall::XenCall;
use xenstore::client::{
    XsPermission, XsdClient, XsdInterface, XS_PERM_NONE, XS_PERM_READ, XS_PERM_READ_WRITE,
};

pub struct XenClient {
    pub store: XsdClient,
    call: XenCall,
}

#[derive(Debug)]
pub struct BlockDeviceRef {
    pub path: String,
    pub major: u32,
    pub minor: u32,
}

#[derive(Debug)]
pub struct DomainDisk<'a> {
    pub vdev: &'a str,
    pub block: &'a BlockDeviceRef,
    pub writable: bool,
}

#[derive(Debug)]
pub struct DomainFilesystem<'a> {
    pub path: &'a str,
    pub tag: &'a str,
}

#[derive(Debug)]
pub struct DomainNetworkInterface<'a> {
    pub mac: &'a str,
    pub mtu: u32,
    pub bridge: Option<&'a str>,
    pub script: Option<&'a str>,
}

#[derive(Debug)]
pub struct DomainConsole {}

#[derive(Debug)]
pub struct DomainEventChannel<'a> {
    pub name: &'a str,
}

#[derive(Debug)]
pub struct DomainConfig<'a> {
    pub backend_domid: u32,
    pub name: &'a str,
    pub max_vcpus: u32,
    pub mem_mb: u64,
    pub kernel_path: &'a str,
    pub initrd_path: &'a str,
    pub cmdline: &'a str,
    pub disks: Vec<DomainDisk<'a>>,
    pub consoles: Vec<DomainConsole>,
    pub vifs: Vec<DomainNetworkInterface<'a>>,
    pub filesystems: Vec<DomainFilesystem<'a>>,
    pub event_channels: Vec<DomainEventChannel<'a>>,
    pub extra_keys: Vec<(String, String)>,
    pub extra_rw_paths: Vec<String>,
}

impl XenClient {
    pub async fn open() -> Result<XenClient> {
        let store = XsdClient::open().await?;
        let call = XenCall::open()?;
        Ok(XenClient { store, call })
    }

    pub async fn create(&mut self, config: &DomainConfig<'_>) -> Result<u32> {
        let domain = CreateDomain {
            max_vcpus: config.max_vcpus,
            ..Default::default()
        };
        let domid = self.call.create_domain(domain)?;
        match self.init(domid, &domain, config).await {
            Ok(_) => Ok(domid),
            Err(err) => {
                // ignore since destroying a domain is best
                // effort when an error occurs
                let _ = self.destroy(domid).await;
                Err(err)
            }
        }
    }

    async fn init(
        &mut self,
        domid: u32,
        domain: &CreateDomain,
        config: &DomainConfig<'_>,
    ) -> Result<()> {
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
            tx.write_string(format!("{}/name", dom_path).as_str(), config.name)
                .await?;
            tx.write_string(format!("{}/name", vm_path).as_str(), config.name)
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

        self.call.set_max_vcpus(domid, config.max_vcpus)?;
        self.call.set_max_mem(domid, config.mem_mb * 1024)?;
        let image_loader = ElfImageLoader::load_file_kernel(config.kernel_path)?;

        let console_evtchn: u32;
        let xenstore_evtchn: u32;
        let console_mfn: u64;
        let xenstore_mfn: u64;

        {
            let mut boot = BootSetup::new(&self.call, domid);
            let mut arch = X86BootSetup::new();
            let initrd = read(config.initrd_path)?;
            let mut state = boot.initialize(
                &mut arch,
                &image_loader,
                initrd.as_slice(),
                config.max_vcpus,
                config.mem_mb,
            )?;
            boot.boot(&mut arch, &mut state, config.cmdline)?;
            console_evtchn = state.console_evtchn;
            xenstore_evtchn = state.store_evtchn;
            console_mfn = boot.phys.p2m[state.console_segment.pfn as usize];
            xenstore_mfn = boot.phys.p2m[state.xenstore_segment.pfn as usize];
        }

        {
            let tx = self.store.transaction().await?;
            tx.write_string(format!("{}/image/os_type", vm_path).as_str(), "linux")
                .await?;
            tx.write_string(
                format!("{}/image/kernel", vm_path).as_str(),
                config.kernel_path,
            )
            .await?;
            tx.write_string(
                format!("{}/image/ramdisk", vm_path).as_str(),
                config.initrd_path,
            )
            .await?;
            tx.write_string(
                format!("{}/image/cmdline", vm_path).as_str(),
                config.cmdline,
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
        self.console_device_add(
            &dom_path,
            &backend_dom_path,
            config.backend_domid,
            domid,
            0,
            Some(console_evtchn),
            Some(console_mfn),
        )
        .await?;

        for (index, _) in config.consoles.iter().enumerate() {
            self.console_device_add(
                &dom_path,
                &backend_dom_path,
                config.backend_domid,
                domid,
                index + 1,
                None,
                None,
            )
            .await?;
        }

        for (index, disk) in config.disks.iter().enumerate() {
            self.disk_device_add(
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
                &dom_path,
                &backend_dom_path,
                config.backend_domid,
                domid,
                index,
                vif,
            )
            .await?;
        }

        for channel in &config.event_channels {
            let id = self
                .call
                .evtchn_alloc_unbound(domid, config.backend_domid)?;
            let channel_path = format!("{}/evtchn/{}", dom_path, channel.name);
            self.store
                .write_string(&format!("{}/name", channel_path), channel.name)
                .await?;
            self.store
                .write_string(&format!("{}/channel", channel_path), &id.to_string())
                .await?;
        }

        self.call.unpause_domain(domid)?;
        Ok(())
    }

    async fn disk_device_add(
        &mut self,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        disk: &DomainDisk<'_>,
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

    #[allow(clippy::too_many_arguments, clippy::unnecessary_unwrap)]
    async fn console_device_add(
        &mut self,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        port: Option<u32>,
        mfn: Option<u64>,
    ) -> Result<()> {
        let mut backend_entries = vec![
            ("frontend-id", domid.to_string()),
            ("online", "1".to_string()),
            ("state", "1".to_string()),
            ("protocol", "vt100".to_string()),
        ];

        let mut frontend_entries = vec![
            ("backend-id", backend_domid.to_string()),
            ("limit", "1048576".to_string()),
            ("output", "pty".to_string()),
            ("tty", "".to_string()),
        ];

        if index == 0 {
            frontend_entries.push(("type", "xenconsoled".to_string()));
        } else {
            frontend_entries.push(("type", "ioemu".to_string()));
            backend_entries.push(("connection", "pty".to_string()));
            backend_entries.push(("output", "pty".to_string()));
        }

        if port.is_some() && mfn.is_some() {
            frontend_entries.extend_from_slice(&[
                ("port", port.unwrap().to_string()),
                ("ring-ref", mfn.unwrap().to_string()),
            ]);
        } else {
            frontend_entries.extend_from_slice(&[
                ("state", "1".to_string()),
                ("protocol", "vt100".to_string()),
            ]);
        }

        self.device_add(
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
        Ok(())
    }

    async fn fs_9p_device_add(
        &mut self,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        filesystem: &DomainFilesystem<'_>,
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
        &mut self,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        vif: &DomainNetworkInterface<'_>,
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
            backend_items.extend_from_slice(&[("bridge", vif.bridge.unwrap().to_string())]);
        }

        if vif.script.is_some() {
            backend_items.extend_from_slice(&[
                ("script", vif.script.unwrap().to_string()),
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

    #[allow(clippy::too_many_arguments)]
    async fn device_add(
        &mut self,
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

        let tx = self.store.transaction().await?;
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
        tx.commit().await?;
        Ok(())
    }

    pub async fn destroy(&mut self, domid: u32) -> Result<()> {
        if let Err(err) = self.destroy_store(domid).await {
            warn!("failed to destroy store for domain {}: {}", domid, err);
        }
        self.call.destroy_domain(domid)?;
        Ok(())
    }

    async fn destroy_store(&mut self, domid: u32) -> Result<()> {
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
            tx.commit().await?;

            let mut count: u32 = 0;
            loop {
                if count >= 100 {
                    warn!("unable to safely destroy backend: {}", backend);
                    break;
                }
                let Some(state) = self.store.read_string(&state_path).await? else {
                    break;
                };
                let state = i64::from_str(&state).unwrap_or(-1);
                if state == 6 {
                    break;
                }
                thread::sleep(Duration::from_millis(100));
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

    pub async fn get_console_path(&mut self, domid: u32) -> Result<String> {
        let dom_path = self.store.get_domain_path(domid).await?;
        let console_tty_path = format!("{}/console/tty", dom_path);
        let mut tty: Option<String> = None;
        for _ in 0..5 {
            tty = self.store.read_string(&console_tty_path).await?;
            if tty.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(200));
        }
        let Some(tty) = tty else {
            return Err(Error::TtyNotFound);
        };
        Ok(tty)
    }
}
