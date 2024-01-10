use crate::sys::{
    ArchDomainConfig, CreateDomain, DomCtl, DomCtlValue, GetDomainInfo, MaxMem, MaxVcpus,
    HYPERVISOR_DOMCTL, XEN_DOMCTL_CREATEDOMAIN, XEN_DOMCTL_GETDOMAININFO,
    XEN_DOMCTL_INTERFACE_VERSION, XEN_DOMCTL_MAX_MEM, XEN_DOMCTL_MAX_VCPUS,
};
use crate::{XenCall, XenCallError};
use std::ffi::c_ulong;
use std::ptr::addr_of;

pub struct DomainControl<'a> {
    call: &'a XenCall,
}

pub struct CreatedDomain {
    pub domid: u32,
}

impl DomainControl<'_> {
    pub fn new(call: &XenCall) -> DomainControl {
        DomainControl { call }
    }

    pub fn get_domain_info(&self, domid: u32) -> Result<GetDomainInfo, XenCallError> {
        let domctl = DomCtl {
            cmd: XEN_DOMCTL_GETDOMAININFO,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                get_domain_info: GetDomainInfo {
                    domid,
                    pad1: 0,
                    flags: 0,
                    total_pages: 0,
                    max_pages: 0,
                    outstanding_pages: 0,
                    shr_pages: 0,
                    paged_pages: 0,
                    shared_info_frame: 0,
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
            .hypercall1(HYPERVISOR_DOMCTL, addr_of!(domctl) as c_ulong)?;
        Ok(unsafe { domctl.value.get_domain_info })
    }

    pub fn create_domain(
        &self,
        create_domain: CreateDomain,
    ) -> Result<CreatedDomain, XenCallError> {
        let domctl = DomCtl {
            cmd: XEN_DOMCTL_CREATEDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid: 0,
            value: DomCtlValue { create_domain },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of!(domctl) as c_ulong)?;
        Ok(CreatedDomain {
            domid: domctl.domid,
        })
    }

    pub fn set_max_mem(&mut self, domid: u32, memkb: u64) -> Result<(), XenCallError> {
        let domctl = DomCtl {
            cmd: XEN_DOMCTL_MAX_MEM,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                max_mem: MaxMem { max_memkb: memkb },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn set_max_vcpus(&mut self, domid: u32, max_vcpus: u32) -> Result<(), XenCallError> {
        let domctl = DomCtl {
            cmd: XEN_DOMCTL_MAX_VCPUS,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue {
                max_cpus: MaxVcpus { max_vcpus },
            },
        };
        self.call
            .hypercall1(HYPERVISOR_DOMCTL, addr_of!(domctl) as c_ulong)?;
        Ok(())
    }
}
