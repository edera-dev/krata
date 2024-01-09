use crate::sys::{
    ArchDomainConfig, CreateDomain, DomCtl, DomCtlValue, GetDomainInfo, HYPERVISOR_DOMCTL,
    XEN_DOMCTL_CREATEDOMAIN, XEN_DOMCTL_GETDOMAININFO, XEN_DOMCTL_INTERFACE_VERSION,
};
use crate::{XenCall, XenCallError};
use std::ffi::c_ulong;
use std::ptr::addr_of;

pub struct DomainControl<'a> {
    call: &'a mut XenCall,
}

pub struct CreatedDomain {
    pub domid: u32,
}

impl DomainControl<'_> {
    pub fn new(call: &mut XenCall) -> DomainControl {
        DomainControl { call }
    }

    pub fn get_domain_info(&mut self, domid: u32) -> Result<GetDomainInfo, XenCallError> {
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
        &mut self,
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
}
