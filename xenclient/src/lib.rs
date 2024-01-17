pub mod boot;
pub mod create;
pub mod elfloader;
pub mod mem;
pub mod sys;
mod x86;

use crate::boot::BootSetup;
use crate::create::DomainConfig;
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
use xenstore::client::{XsPermissions, XsdClient, XsdInterface};

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

impl XenClient {
    pub fn open() -> Result<XenClient, XenClientError> {
        let store = XsdClient::open()?;
        let call = XenCall::open()?;
        Ok(XenClient { store, call })
    }

    pub fn create(&mut self, config: DomainConfig) -> Result<u32, XenClientError> {
        let domain = CreateDomain {
            max_vcpus: config.max_vcpus,
            ..Default::default()
        };
        let domid = self.call.create_domain(domain)?;
        let dom_path = self.store.get_domain_path(domid)?;
        let uuid_string = Uuid::from_bytes(domain.handle).to_string();
        let vm_path = format!("/vm/{}", uuid_string);
        let libxl_path = format!("/libxl/{}", domid);

        let ro_perm = XsPermissions { id: 0, perms: 0 };

        let rw_perm = XsPermissions { id: 0, perms: 0 };

        let no_perm = XsPermissions { id: 0, perms: 0 };

        {
            let mut tx = self.store.transaction()?;

            tx.rm(dom_path.as_str())?;
            tx.mknod(dom_path.as_str(), &ro_perm)?;

            tx.rm(vm_path.as_str())?;
            tx.mknod(vm_path.as_str(), &ro_perm)?;

            tx.rm(libxl_path.as_str())?;
            tx.mknod(vm_path.as_str(), &no_perm)?;
            tx.mknod(format!("{}/device", vm_path).as_str(), &no_perm)?;

            tx.write_string(format!("{}/vm", dom_path).as_str(), &vm_path)?;

            tx.mknod(format!("{}/cpu", dom_path).as_str(), &ro_perm)?;
            tx.mknod(format!("{}/memory", dom_path).as_str(), &ro_perm)?;

            tx.mknod(format!("{}/control", dom_path).as_str(), &ro_perm)?;

            tx.mknod(format!("{}/control/shutdown", dom_path).as_str(), &rw_perm)?;
            tx.mknod(
                format!("{}/control/feature-poweroff", dom_path).as_str(),
                &rw_perm,
            )?;
            tx.mknod(
                format!("{}/control/feature-reboot", dom_path).as_str(),
                &rw_perm,
            )?;
            tx.mknod(
                format!("{}/control/feature-suspend", dom_path).as_str(),
                &rw_perm,
            )?;
            tx.mknod(format!("{}/control/sysrq", dom_path).as_str(), &rw_perm)?;

            tx.mknod(format!("{}/data", dom_path).as_str(), &rw_perm)?;
            tx.mknod(format!("{}/drivers", dom_path).as_str(), &rw_perm)?;
            tx.mknod(format!("{}/feature", dom_path).as_str(), &rw_perm)?;
            tx.mknod(format!("{}/attr", dom_path).as_str(), &rw_perm)?;
            tx.mknod(format!("{}/error", dom_path).as_str(), &rw_perm)?;

            tx.write_string(
                format!("{}/uuid", vm_path).as_str(),
                &Uuid::from_bytes(domain.handle).to_string(),
            )?;
            tx.write_string(format!("{}/name", vm_path).as_str(), "mycelium")?;
            tx.write_string(format!("{}/type", libxl_path).as_str(), "pv")?;
            tx.commit()?;
        }

        self.call.set_max_vcpus(domid, config.max_vcpus)?;
        self.call.set_max_mem(domid, config.mem_mb * 1024)?;
        let image_loader = ElfImageLoader::load_file_kernel(config.kernel_path.as_str())?;

        let console_evtchn: u32;
        let xenstore_evtchn: u32;
        let console_mfn: u64;
        let xenstore_mfn: u64;

        {
            let mut boot = BootSetup::new(&self.call, domid);
            let mut arch = X86BootSetup::new();
            let initrd = read(config.initrd_path.as_str())?;
            let mut state = boot.initialize(
                &mut arch,
                &image_loader,
                initrd.as_slice(),
                config.max_vcpus,
                config.mem_mb,
            )?;
            boot.boot(&mut arch, &mut state, config.cmdline.as_str())?;
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
                &config.kernel_path,
            )?;
            tx.write_string(
                format!("{}/image/ramdisk", vm_path).as_str(),
                &config.initrd_path,
            )?;
            tx.write_string(
                format!("{}/image/cmdline", vm_path).as_str(),
                &config.cmdline,
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
                tx.write_string(
                    format!("{}/cpu/{}/availability", dom_path, i).as_str(),
                    "online",
                )?;
            }
            tx.commit()?;
        }

        self.console_device_add(&dom_path.to_string(), domid, console_evtchn, console_mfn)?;
        self.store
            .introduce_domain(domid, xenstore_mfn, xenstore_evtchn)?;
        self.call.unpause_domain(domid)?;

        Ok(domid)
    }

    fn console_device_add(
        &mut self,
        dom_path: &String,
        domid: u32,
        port: u32,
        mfn: u64,
    ) -> Result<(), XenClientError> {
        let frontend_path = format!("{}/console", dom_path);
        let backend_path = format!("{}/backend/console/{}/{}", dom_path, domid, 0);
        let mut tx = self.store.transaction()?;
        tx.write_string(
            format!("{}/frontend-id", backend_path).as_str(),
            &domid.to_string(),
        )?;
        tx.write_string(format!("{}/online", backend_path).as_str(), "1")?;
        tx.write_string(format!("{}/state", backend_path).as_str(), "1")?;
        tx.write_string(format!("{}/protocol", backend_path).as_str(), "vt100")?;

        tx.write_string(format!("{}/backend-id", frontend_path).as_str(), "0")?;
        tx.write_string(format!("{}/limit", frontend_path).as_str(), "1048576")?;
        tx.write_string(format!("{}/type", frontend_path).as_str(), "xenconsoled")?;
        tx.write_string(format!("{}/output", frontend_path).as_str(), "pty")?;
        tx.write_string(format!("{}/tty", frontend_path).as_str(), "")?;
        tx.write_string(
            format!("{}/port", frontend_path).as_str(),
            &port.to_string(),
        )?;
        tx.write_string(
            format!("{}/ring-ref", frontend_path).as_str(),
            &mfn.to_string(),
        )?;
        tx.commit()?;
        Ok(())
    }
}
