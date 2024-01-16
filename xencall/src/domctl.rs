use crate::sys::{
    AddressSize, ArchDomainConfig, CreateDomain, DomCtl, DomCtlValue, DomCtlVcpuContext,
    GetDomainInfo, GetPageFrameInfo3, HypercallInit, MaxMem, MaxVcpus, VcpuGuestContext,
    VcpuGuestContextAny, HYPERVISOR_DOMCTL, XEN_DOMCTL_CREATEDOMAIN, XEN_DOMCTL_DESTROYDOMAIN,
    XEN_DOMCTL_GETDOMAININFO, XEN_DOMCTL_GETPAGEFRAMEINFO3, XEN_DOMCTL_GETVCPUCONTEXT,
    XEN_DOMCTL_HYPERCALL_INIT, XEN_DOMCTL_INTERFACE_VERSION, XEN_DOMCTL_MAX_MEM,
    XEN_DOMCTL_MAX_VCPUS, XEN_DOMCTL_PAUSEDOMAIN, XEN_DOMCTL_SETVCPUCONTEXT,
    XEN_DOMCTL_SET_ADDRESS_SIZE,
};
use crate::{XenCall, XenCallError};
use log::trace;
use std::ffi::c_ulong;
use std::os::fd::AsRawFd;
use std::ptr::addr_of_mut;
use std::slice;

pub struct DomainControl<'a> {
    call: &'a XenCall,
}

impl DomainControl<'_> {
    pub fn new(call: &XenCall) -> DomainControl {
        DomainControl { call }
    }

    pub fn get_domain_info(&self, domid: u32) -> Result<GetDomainInfo, XenCallError> {
        trace!(
            "domctl fd={} get_domain_info domid={}",
            self.call.handle.as_raw_fd(),
            domid
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_GETDOMAININFO,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                get_domain_info: GetDomainInfo {
                    domid: 0,
                    pad1: 0,
                    flags: 0,
                    total_pages: 0,
                    max_pages: 0,
                    outstanding_pages: 0,
                    shr_pages: 0,
                    paged_pages: 0,
                    shared_info_frame: 0,
                    cpu_time: 0,
                    number_online_vcpus: 0,
                    max_vcpu_id: 0,
                    ssidref: 0,
                    handle: [0; 16],
                    cpupool: 0,
                    gpaddr_bits: 0,
                    pad2: [0; 7],
                    arch: ArchDomainConfig {
                        emulation_flags: 0,
                        misc_flags: 0,
                    },
                },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(unsafe { domctl.value.get_domain_info })
    }

    pub fn create_domain(&self, create_domain: CreateDomain) -> Result<u32, XenCallError> {
        trace!(
            "domctl fd={} create_domain create_domain={:?}",
            self.call.handle.as_raw_fd(),
            create_domain
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_CREATEDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid: 0,
            value: DomCtlValue { create_domain },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(domctl.domid)
    }

    pub fn pause_domain(&self, domid: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} pause_domain domid={:?}",
            self.call.handle.as_raw_fd(),
            domid,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_PAUSEDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn set_max_mem(&self, domid: u32, memkb: u64) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} set_max_mem domid={} memkb={}",
            self.call.handle.as_raw_fd(),
            domid,
            memkb
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_MAX_MEM,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                max_mem: MaxMem { max_memkb: memkb },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn set_max_vcpus(&self, domid: u32, max_vcpus: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} set_max_vcpus domid={} max_vcpus={}",
            self.call.handle.as_raw_fd(),
            domid,
            max_vcpus
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_MAX_VCPUS,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                max_cpus: MaxVcpus { max_vcpus },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn set_address_size(&self, domid: u32, size: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} set_address_size domid={} size={}",
            self.call.handle.as_raw_fd(),
            domid,
            size,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_SET_ADDRESS_SIZE,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                address_size: AddressSize { size },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn get_vcpu_context(
        &self,
        domid: u32,
        vcpu: u32,
    ) -> Result<VcpuGuestContext, XenCallError> {
        trace!(
            "domctl fd={} get_vcpu_context domid={}",
            self.call.handle.as_raw_fd(),
            domid,
        );
        let mut wrapper = VcpuGuestContextAny {
            value: VcpuGuestContext::default(),
        };
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_GETVCPUCONTEXT,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                vcpu_context: DomCtlVcpuContext {
                    vcpu,
                    ctx: addr_of_mut!(wrapper) as c_ulong,
                },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(unsafe { wrapper.value })
    }

    pub fn set_vcpu_context(
        &self,
        domid: u32,
        vcpu: u32,
        context: &VcpuGuestContext,
    ) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} set_vcpu_context domid={} context={:?}",
            self.call.handle.as_raw_fd(),
            domid,
            context,
        );

        let mut value = VcpuGuestContextAny { value: *context };
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_SETVCPUCONTEXT,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                vcpu_context: DomCtlVcpuContext {
                    vcpu,
                    ctx: addr_of_mut!(value) as c_ulong,
                },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn get_page_frame_info(
        &self,
        domid: u32,
        frames: &[u64],
    ) -> Result<Vec<u64>, XenCallError> {
        let mut buffer: Vec<u64> = frames.to_vec();
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_GETPAGEFRAMEINFO3,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                get_page_frame_info: GetPageFrameInfo3 {
                    num: buffer.len() as u64,
                    array: buffer.as_mut_ptr() as c_ulong,
                },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        let slice = unsafe {
            slice::from_raw_parts_mut(
                domctl.value.get_page_frame_info.array as *mut u64,
                domctl.value.get_page_frame_info.num as usize,
            )
        };
        Ok(slice.to_vec())
    }

    pub fn hypercall_init(&self, domid: u32, gmfn: u64) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} hypercall_init domid={} gmfn={}",
            self.call.handle.as_raw_fd(),
            domid,
            gmfn
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_HYPERCALL_INIT,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                hypercall_init: HypercallInit { gmfn },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn destroy_domain(&self, domid: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} destroy_domain domid={}",
            self.call.handle.as_raw_fd(),
            domid
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_DESTROYDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }
}
