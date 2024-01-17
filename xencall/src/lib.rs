pub mod sys;

use crate::sys::{
    AddressSize, ArchDomainConfig, CreateDomain, DomCtl, DomCtlValue, DomCtlVcpuContext,
    EvtChnAllocUnbound, GetDomainInfo, GetPageFrameInfo3, Hypercall, HypercallInit, MaxMem,
    MaxVcpus, MemoryMap, MemoryReservation, MmapBatch, MmapResource, MmuExtOp, MultiCallEntry,
    VcpuGuestContext, VcpuGuestContextAny, XenCapabilitiesInfo, HYPERVISOR_DOMCTL,
    HYPERVISOR_EVENT_CHANNEL_OP, HYPERVISOR_MEMORY_OP, HYPERVISOR_MMUEXT_OP, HYPERVISOR_MULTICALL,
    HYPERVISOR_XEN_VERSION, XENVER_CAPABILITIES, XEN_DOMCTL_CREATEDOMAIN, XEN_DOMCTL_DESTROYDOMAIN,
    XEN_DOMCTL_GETDOMAININFO, XEN_DOMCTL_GETPAGEFRAMEINFO3, XEN_DOMCTL_GETVCPUCONTEXT,
    XEN_DOMCTL_HYPERCALL_INIT, XEN_DOMCTL_INTERFACE_VERSION, XEN_DOMCTL_MAX_MEM,
    XEN_DOMCTL_MAX_VCPUS, XEN_DOMCTL_PAUSEDOMAIN, XEN_DOMCTL_SETVCPUCONTEXT,
    XEN_DOMCTL_SET_ADDRESS_SIZE, XEN_DOMCTL_UNPAUSEDOMAIN, XEN_MEM_MEMORY_MAP,
    XEN_MEM_POPULATE_PHYSMAP,
};
use libc::{c_int, mmap, MAP_FAILED, MAP_SHARED, PROT_READ, PROT_WRITE};
use log::trace;
use nix::errno::Errno;
use std::error::Error;
use std::ffi::{c_long, c_uint, c_ulong, c_void};
use std::fmt::{Display, Formatter};
use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::ptr::addr_of_mut;
use std::slice;

pub struct XenCall {
    pub handle: File,
}

#[derive(Debug)]
pub struct XenCallError {
    message: String,
}

impl XenCallError {
    pub fn new(msg: &str) -> XenCallError {
        XenCallError {
            message: msg.to_string(),
        }
    }
}

impl Display for XenCallError {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for XenCallError {
    fn description(&self) -> &str {
        &self.message
    }
}

impl From<std::io::Error> for XenCallError {
    fn from(value: std::io::Error) -> Self {
        XenCallError::new(value.to_string().as_str())
    }
}

impl From<Errno> for XenCallError {
    fn from(value: Errno) -> Self {
        XenCallError::new(value.to_string().as_str())
    }
}

impl XenCall {
    pub fn open() -> Result<XenCall, XenCallError> {
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/privcmd")?;
        Ok(XenCall { handle: file })
    }

    pub fn mmap(&self, addr: u64, len: u64) -> Option<u64> {
        trace!(
            "call fd={} mmap addr={:#x} len={}",
            self.handle.as_raw_fd(),
            addr,
            len
        );
        unsafe {
            let ptr = mmap(
                addr as *mut c_void,
                len as usize,
                PROT_READ | PROT_WRITE,
                MAP_SHARED,
                self.handle.as_raw_fd(),
                0,
            );
            if ptr == MAP_FAILED {
                None
            } else {
                Some(ptr as u64)
            }
        }
    }

    pub fn hypercall(&self, op: c_ulong, arg: [c_ulong; 5]) -> Result<c_long, XenCallError> {
        trace!(
            "call fd={} hypercall op={:#x}, arg={:?}",
            self.handle.as_raw_fd(),
            op,
            arg
        );
        unsafe {
            let mut call = Hypercall { op, arg };
            let result = sys::hypercall(self.handle.as_raw_fd(), &mut call)?;
            Ok(result as c_long)
        }
    }

    pub fn hypercall0(&self, op: c_ulong) -> Result<c_long, XenCallError> {
        self.hypercall(op, [0, 0, 0, 0, 0])
    }

    pub fn hypercall1(&self, op: c_ulong, arg1: c_ulong) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, 0, 0, 0, 0])
    }

    pub fn hypercall2(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, 0, 0, 0])
    }

    pub fn hypercall3(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, 0, 0])
    }

    pub fn hypercall4(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, 0])
    }

    pub fn hypercall5(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
        arg5: c_ulong,
    ) -> Result<c_long, XenCallError> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, arg5])
    }

    pub fn multicall(&self, calls: &mut [MultiCallEntry]) -> Result<(), XenCallError> {
        trace!(
            "call fd={} multicall calls={:?}",
            self.handle.as_raw_fd(),
            calls
        );
        self.hypercall2(
            HYPERVISOR_MULTICALL,
            calls.as_mut_ptr() as c_ulong,
            calls.len() as c_ulong,
        )?;
        Ok(())
    }

    pub fn map_resource(
        &self,
        domid: u32,
        typ: u32,
        id: u32,
        idx: u32,
        num: u64,
        addr: u64,
    ) -> Result<(), XenCallError> {
        let mut resource = MmapResource {
            dom: domid as u16,
            typ,
            id,
            idx,
            num,
            addr,
        };
        unsafe {
            sys::mmap_resource(self.handle.as_raw_fd(), &mut resource)?;
        }
        Ok(())
    }

    pub fn mmap_batch(
        &self,
        domid: u32,
        num: u64,
        addr: u64,
        mfns: Vec<u64>,
    ) -> Result<c_long, XenCallError> {
        trace!(
            "call fd={} mmap_batch domid={} num={} addr={:#x} mfns={:?}",
            self.handle.as_raw_fd(),
            domid,
            num,
            addr,
            mfns
        );
        unsafe {
            let mut mfns = mfns.clone();
            let mut errors = vec![0i32; mfns.len()];
            let mut batch = MmapBatch {
                num: num as u32,
                domid: domid as u16,
                addr,
                mfns: mfns.as_mut_ptr(),
                errors: errors.as_mut_ptr(),
            };
            let result = sys::mmapbatch(self.handle.as_raw_fd(), &mut batch)?;
            Ok(result as c_long)
        }
    }

    pub fn get_version_capabilities(&self) -> Result<XenCapabilitiesInfo, XenCallError> {
        trace!(
            "call fd={} get_version_capabilities",
            self.handle.as_raw_fd()
        );
        let mut info = XenCapabilitiesInfo {
            capabilities: [0; 1024],
        };
        self.hypercall2(
            HYPERVISOR_XEN_VERSION,
            XENVER_CAPABILITIES,
            addr_of_mut!(info) as c_ulong,
        )?;
        Ok(info)
    }

    pub fn evtchn_op(&self, cmd: c_int, arg: u64) -> Result<(), XenCallError> {
        self.hypercall2(HYPERVISOR_EVENT_CHANNEL_OP, cmd as c_ulong, arg)?;
        Ok(())
    }

    pub fn evtchn_alloc_unbound(&self, domid: u32, remote_domid: u32) -> Result<u32, XenCallError> {
        let mut alloc_unbound = EvtChnAllocUnbound {
            dom: domid as u16,
            remote_dom: remote_domid as u16,
            port: 0,
        };
        self.evtchn_op(6, addr_of_mut!(alloc_unbound) as c_ulong)?;
        Ok(alloc_unbound.port)
    }

    pub fn get_domain_info(&self, domid: u32) -> Result<GetDomainInfo, XenCallError> {
        trace!(
            "domctl fd={} get_domain_info domid={}",
            self.handle.as_raw_fd(),
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(unsafe { domctl.value.get_domain_info })
    }

    pub fn create_domain(&self, create_domain: CreateDomain) -> Result<u32, XenCallError> {
        trace!(
            "domctl fd={} create_domain create_domain={:?}",
            self.handle.as_raw_fd(),
            create_domain
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_CREATEDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid: 0,
            value: DomCtlValue { create_domain },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(domctl.domid)
    }

    pub fn pause_domain(&self, domid: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} pause_domain domid={:?}",
            self.handle.as_raw_fd(),
            domid,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_PAUSEDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn unpause_domain(&self, domid: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} unpause_domain domid={:?}",
            self.handle.as_raw_fd(),
            domid,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_UNPAUSEDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn set_max_mem(&self, domid: u32, memkb: u64) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} set_max_mem domid={} memkb={}",
            self.handle.as_raw_fd(),
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn set_max_vcpus(&self, domid: u32, max_vcpus: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} set_max_vcpus domid={} max_vcpus={}",
            self.handle.as_raw_fd(),
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn set_address_size(&self, domid: u32, size: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} set_address_size domid={} size={}",
            self.handle.as_raw_fd(),
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn get_vcpu_context(
        &self,
        domid: u32,
        vcpu: u32,
    ) -> Result<VcpuGuestContext, XenCallError> {
        trace!(
            "domctl fd={} get_vcpu_context domid={}",
            self.handle.as_raw_fd(),
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
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
            self.handle.as_raw_fd(),
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
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
            self.handle.as_raw_fd(),
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
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn destroy_domain(&self, domid: u32) -> Result<(), XenCallError> {
        trace!(
            "domctl fd={} destroy_domain domid={}",
            self.handle.as_raw_fd(),
            domid
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_DESTROYDOMAIN,
            interface_version: XEN_DOMCTL_INTERFACE_VERSION,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)?;
        Ok(())
    }

    pub fn get_memory_map(&self, size_of_entry: usize) -> Result<Vec<u8>, XenCallError> {
        let mut memory_map = MemoryMap {
            count: 0,
            buffer: 0,
        };
        self.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_MEMORY_MAP as c_ulong,
            addr_of_mut!(memory_map) as c_ulong,
        )?;
        let mut buffer = vec![0u8; memory_map.count as usize * size_of_entry];
        memory_map.buffer = buffer.as_mut_ptr() as c_ulong;
        self.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_MEMORY_MAP as c_ulong,
            addr_of_mut!(memory_map) as c_ulong,
        )?;
        Ok(buffer)
    }

    pub fn populate_physmap(
        &self,
        domid: u32,
        nr_extents: u64,
        extent_order: u32,
        mem_flags: u32,
        extent_starts: &[u64],
    ) -> Result<Vec<u64>, XenCallError> {
        trace!("memory fd={} populate_physmap domid={} nr_extents={} extent_order={} mem_flags={} extent_starts={:?}", self.handle.as_raw_fd(), domid, nr_extents, extent_order, mem_flags, extent_starts);
        let mut extent_starts = extent_starts.to_vec();
        let ptr = extent_starts.as_mut_ptr();

        let mut reservation = MemoryReservation {
            extent_start: ptr as c_ulong,
            nr_extents,
            extent_order,
            mem_flags,
            domid: domid as u16,
        };

        let calls = &mut [MultiCallEntry {
            op: HYPERVISOR_MEMORY_OP,
            result: 0,
            args: [
                XEN_MEM_POPULATE_PHYSMAP as c_ulong,
                addr_of_mut!(reservation) as c_ulong,
                0,
                0,
                0,
                0,
            ],
        }];
        self.multicall(calls)?;
        let code = calls[0].result;
        if code > !0xfff {
            return Err(XenCallError::new(
                format!("failed to populate physmap: {:#x}", code).as_str(),
            ));
        }
        if code as usize > extent_starts.len() {
            return Err(XenCallError::new("failed to populate physmap"));
        }
        let extents = extent_starts[0..code as usize].to_vec();
        Ok(extents)
    }

    pub fn mmuext(
        &self,
        domid: u32,
        cmd: c_uint,
        arg1: u64,
        arg2: u64,
    ) -> Result<(), XenCallError> {
        let mut ops = MmuExtOp { cmd, arg1, arg2 };

        self.hypercall4(
            HYPERVISOR_MMUEXT_OP,
            addr_of_mut!(ops) as c_ulong,
            1,
            0,
            domid as c_ulong,
        )
        .map(|_| ())
    }
}
