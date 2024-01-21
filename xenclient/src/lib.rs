pub mod boot;
pub mod elfloader;
pub mod mem;
pub mod sys;
mod x86;

use crate::boot::BootSetup;
use crate::elfloader::ElfImageLoader;
use crate::x86::X86BootSetup;
use std::error::Error;
use std::fmt::{Display, Formatter};
use std::fs::read;
use std::string::FromUtf8Error;
use uuid::Uuid;
use xencall::sys::CreateDomain;
use xencall::{XenCall, XenCallError};
use xenevtchn::EventChannelError;
use xenstore::bus::XsdBusError;
use xenstore::client::{
    XsPermission, XsdClient, XsdInterface, XS_PERM_NONE, XS_PERM_READ, XS_PERM_READ_WRITE,
};

pub struct XenClient {
    store: XsdClient,
    call: XenCall,
}

#[derive(Debug)]
pub struct XenClientError {
    message: String,
}

impl XenClientError {
    pub fn new(msg: &str) -> XenClientError {
        XenClientError {
            message: msg.to_string(),
        }
    }
}

impl Display for XenClientError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for XenClientError {
    fn description(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for XenClientError {
    fn from(value: std::io::Error) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<XsdBusError> for XenClientError {
    fn from(value: XsdBusError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<XenCallError> for XenClientError {
    fn from(value: XenCallError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<FromUtf8Error> for XenClientError {
    fn from(value: FromUtf8Error) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

impl From<EventChannelError> for XenClientError {
    fn from(value: EventChannelError) -> Self {
        XenClientError::new(value.to_string().as_str())
    }
}

pub struct DomainDisk<'a> {
    pub vdev: &'a str,
    pub pdev: &'a str,
    pub writable: bool,
}

pub struct DomainConfig<'a> {
    pub backend_domid: u32,
    pub name: &'a str,
    pub max_vcpus: u32,
    pub mem_mb: u64,
    pub kernel_path: &'a str,
    pub initrd_path: &'a str,
    pub cmdline: &'a str,
    pub disks: Vec<DomainDisk<'a>>,
}

impl XenClient {
    pub fn open() -> Result<XenClient, XenClientError> {
        let store = XsdClient::open()?;
        let call = XenCall::open()?;
        Ok(XenClient { store, call })
    }

    pub fn create(&mut self, config: &DomainConfig) -> Result<u32, XenClientError> {
        let domain = CreateDomain {
            max_vcpus: config.max_vcpus,
            ..Default::default()
        };
        let domid = self.call.create_domain(domain)?;
        match self.init(domid, &domain, config) {
            Ok(_) => Ok(domid),
            Err(err) => {
                // ignore since destroying a domain is best
                // effort when an error occurs
                let _ = self.call.destroy_domain(domid);
                Err(err)
            }
        }
    }

    fn init(
        &mut self,
        domid: u32,
        domain: &CreateDomain,
        config: &DomainConfig,
    ) -> Result<(), XenClientError> {
        let backend_dom_path = self.store.get_domain_path(0)?;
        let dom_path = self.store.get_domain_path(domid)?;
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
            let mut tx = self.store.transaction()?;

            tx.rm(dom_path.as_str())?;
            tx.mknod(dom_path.as_str(), ro_perm)?;

            tx.rm(vm_path.as_str())?;
            tx.mknod(vm_path.as_str(), ro_perm)?;

            tx.mknod(vm_path.as_str(), no_perm)?;
            tx.mknod(format!("{}/device", vm_path).as_str(), no_perm)?;

            tx.write_string(format!("{}/vm", dom_path).as_str(), &vm_path)?;

            tx.mknod(format!("{}/cpu", dom_path).as_str(), ro_perm)?;
            tx.mknod(format!("{}/memory", dom_path).as_str(), ro_perm)?;

            tx.mknod(format!("{}/control", dom_path).as_str(), ro_perm)?;

            tx.mknod(format!("{}/control/shutdown", dom_path).as_str(), rw_perm)?;
            tx.mknod(
                format!("{}/control/feature-poweroff", dom_path).as_str(),
                rw_perm,
            )?;
            tx.mknod(
                format!("{}/control/feature-reboot", dom_path).as_str(),
                rw_perm,
            )?;
            tx.mknod(
                format!("{}/control/feature-suspend", dom_path).as_str(),
                rw_perm,
            )?;
            tx.mknod(format!("{}/control/sysrq", dom_path).as_str(), rw_perm)?;

            tx.mknod(format!("{}/data", dom_path).as_str(), rw_perm)?;
            tx.mknod(format!("{}/drivers", dom_path).as_str(), rw_perm)?;
            tx.mknod(format!("{}/feature", dom_path).as_str(), rw_perm)?;
            tx.mknod(format!("{}/attr", dom_path).as_str(), rw_perm)?;
            tx.mknod(format!("{}/error", dom_path).as_str(), rw_perm)?;

            tx.write_string(
                format!("{}/uuid", vm_path).as_str(),
                &Uuid::from_bytes(domain.handle).to_string(),
            )?;
            tx.write_string(format!("{}/name", dom_path).as_str(), config.name)?;
            tx.write_string(format!("{}/name", vm_path).as_str(), config.name)?;
            tx.commit()?;
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
            let mut tx = self.store.transaction()?;
            tx.write_string(format!("{}/image/os_type", vm_path).as_str(), "linux")?;
            tx.write_string(
                format!("{}/image/kernel", vm_path).as_str(),
                config.kernel_path,
            )?;
            tx.write_string(
                format!("{}/image/ramdisk", vm_path).as_str(),
                config.initrd_path,
            )?;
            tx.write_string(
                format!("{}/image/cmdline", vm_path).as_str(),
                config.cmdline,
            )?;

            tx.write_string(
                format!("{}/memory/static-max", dom_path).as_str(),
                &(config.mem_mb * 1024).to_string(),
            )?;
            tx.write_string(
                format!("{}/memory/target", dom_path).as_str(),
                &(config.mem_mb * 1024).to_string(),
            )?;
            tx.write_string(format!("{}/memory/videoram", dom_path).as_str(), "0")?;
            tx.write_string(format!("{}/domid", dom_path).as_str(), &domid.to_string())?;
            tx.write_string(
                format!("{}/store/port", dom_path).as_str(),
                &xenstore_evtchn.to_string(),
            )?;
            tx.write_string(
                format!("{}/store/ring-ref", dom_path).as_str(),
                &xenstore_mfn.to_string(),
            )?;
            for i in 0..config.max_vcpus {
                let path = format!("{}/cpu/{}", dom_path, i);
                tx.mkdir(&path)?;
                tx.set_perms(&path, ro_perm)?;
                let path = format!("{}/cpu/{}/availability", dom_path, i);
                tx.write_string(&path, "online")?;
                tx.set_perms(&path, ro_perm)?;
            }
            tx.commit()?;
        }
        if !self
            .store
            .introduce_domain(domid, xenstore_mfn, xenstore_evtchn)?
        {
            return Err(XenClientError::new("failed to introduce domain"));
        }
        self.console_device_add(
            &dom_path,
            &backend_dom_path,
            config.backend_domid,
            domid,
            console_evtchn,
            console_mfn,
        )?;
        for (index, disk) in config.disks.iter().enumerate() {
            self.disk_device_add(
                &dom_path,
                &backend_dom_path,
                config.backend_domid,
                domid,
                index,
                disk,
            )?;
        }
        self.call.unpause_domain(domid)?;
        Ok(())
    }

    fn disk_device_add(
        &mut self,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        index: usize,
        disk: &DomainDisk,
    ) -> Result<(), XenClientError> {
        let id = (202 << 8) | (index << 4) as u64;
        let backend_items: Vec<(&str, String)> = vec![
            ("frontend-id", domid.to_string()),
            ("params", disk.pdev.to_string()),
            ("script", "/etc/xen/scripts/block".to_string()),
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
        )?;
        Ok(())
    }

    fn console_device_add(
        &mut self,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        port: u32,
        mfn: u64,
    ) -> Result<(), XenClientError> {
        let backend_entries = vec![
            ("frontend-id", domid.to_string()),
            ("online", "1".to_string()),
            ("state", "1".to_string()),
            ("protocol", "vt100".to_string()),
        ];

        let frontend_entries = vec![
            ("backend-id", backend_domid.to_string()),
            ("limit", "1048576".to_string()),
            ("type", "xenconsoled".to_string()),
            ("output", "pty".to_string()),
            ("tty", "".to_string()),
            ("port", port.to_string()),
            ("ring-ref", mfn.to_string()),
        ];

        self.device_add(
            "console",
            0,
            dom_path,
            backend_dom_path,
            backend_domid,
            domid,
            frontend_entries,
            backend_entries,
        )?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn device_add(
        &mut self,
        typ: &str,
        id: u64,
        dom_path: &str,
        backend_dom_path: &str,
        backend_domid: u32,
        domid: u32,
        frontend_items: Vec<(&str, String)>,
        backend_items: Vec<(&str, String)>,
    ) -> Result<(), XenClientError> {
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

        let mut tx = self.store.transaction()?;
        tx.mknod(&frontend_path, frontend_perms)?;
        for (p, value) in &frontend_items {
            let path = format!("{}/{}", frontend_path, *p);
            tx.write_string(&path, value)?;
            if !console_zero {
                tx.set_perms(&path, frontend_perms)?;
            }
        }
        tx.mknod(&backend_path, backend_perms)?;
        for (p, value) in &backend_items {
            let path = format!("{}/{}", backend_path, *p);
            tx.write_string(&path, value)?;
        }
        tx.commit()?;
        Ok(())
    }
}
