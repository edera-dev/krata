pub mod error;
pub mod sys;

use crate::error::{Error, Result};
use crate::sys::{
    AddressSize, AssignDevice, CreateDomain, DomCtl, DomCtlValue, DomCtlVcpuContext,
    EvtChnAllocUnbound, GetDomainInfo, GetPageFrameInfo3, HvmContext, HvmParam, Hypercall,
    HypercallInit, IoMemPermission, IoPortPermission, IrqPermission, MaxMem, MaxVcpus, MemoryMap,
    MemoryReservation, MmapBatch, MmapResource, MmuExtOp, MultiCallEntry, PagingMempool,
    PciAssignDevice, XenCapabilitiesInfo, DOMCTL_DEV_PCI, HYPERVISOR_DOMCTL,
    HYPERVISOR_EVENT_CHANNEL_OP, HYPERVISOR_HVM_OP, HYPERVISOR_MEMORY_OP, HYPERVISOR_MMUEXT_OP,
    HYPERVISOR_MULTICALL, HYPERVISOR_XEN_VERSION, XENVER_CAPABILITIES, XEN_DOMCTL_ASSIGN_DEVICE,
    XEN_DOMCTL_CREATEDOMAIN, XEN_DOMCTL_DESTROYDOMAIN, XEN_DOMCTL_GETDOMAININFO,
    XEN_DOMCTL_GETHVMCONTEXT, XEN_DOMCTL_GETPAGEFRAMEINFO3, XEN_DOMCTL_HYPERCALL_INIT,
    XEN_DOMCTL_IOMEM_PERMISSION, XEN_DOMCTL_IOPORT_PERMISSION, XEN_DOMCTL_IRQ_PERMISSION,
    XEN_DOMCTL_MAX_MEM, XEN_DOMCTL_MAX_VCPUS, XEN_DOMCTL_PAUSEDOMAIN, XEN_DOMCTL_SETHVMCONTEXT,
    XEN_DOMCTL_SETVCPUCONTEXT, XEN_DOMCTL_SET_ADDRESS_SIZE, XEN_DOMCTL_SET_PAGING_MEMPOOL_SIZE,
    XEN_DOMCTL_UNPAUSEDOMAIN, XEN_MEM_CLAIM_PAGES, XEN_MEM_MEMORY_MAP, XEN_MEM_POPULATE_PHYSMAP,
};
use libc::{c_int, mmap, MAP_FAILED, MAP_SHARED, PROT_READ, PROT_WRITE};
use log::trace;
use nix::errno::Errno;
use std::ffi::{c_long, c_uint, c_ulong, c_void};
use std::sync::Arc;
use std::time::Duration;
use sys::{
    E820Entry, ForeignMemoryMap, PhysdevMapPirq, VcpuGuestContextAny, HYPERVISOR_PHYSDEV_OP,
    PHYSDEVOP_MAP_PIRQ, XEN_DOMCTL_MAX_INTERFACE_VERSION, XEN_DOMCTL_MIN_INTERFACE_VERSION,
    XEN_MEM_SET_MEMORY_MAP,
};
use tokio::sync::Semaphore;
use tokio::time::sleep;

use std::fs::{File, OpenOptions};
use std::os::fd::AsRawFd;
use std::ptr::{addr_of_mut, null_mut};
use std::slice;

#[derive(Clone)]
pub struct XenCall {
    pub handle: Arc<File>,
    semaphore: Arc<Semaphore>,
    domctl_interface_version: u32,
}

impl XenCall {
    pub fn open(current_domid: u32) -> Result<XenCall> {
        let handle = OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/xen/privcmd")?;
        let domctl_interface_version =
            XenCall::detect_domctl_interface_version(&handle, current_domid)?;
        Ok(XenCall {
            handle: Arc::new(handle),
            semaphore: Arc::new(Semaphore::new(1)),
            domctl_interface_version,
        })
    }

    fn detect_domctl_interface_version(handle: &File, current_domid: u32) -> Result<u32> {
        for version in XEN_DOMCTL_MIN_INTERFACE_VERSION..XEN_DOMCTL_MAX_INTERFACE_VERSION + 1 {
            let mut domctl = DomCtl {
                cmd: XEN_DOMCTL_GETDOMAININFO,
                interface_version: version,
                domid: current_domid,
                value: DomCtlValue {
                    get_domain_info: GetDomainInfo::default(),
                },
            };
            unsafe {
                let mut call = Hypercall {
                    op: HYPERVISOR_DOMCTL,
                    arg: [addr_of_mut!(domctl) as u64, 0, 0, 0, 0],
                };
                let result = sys::hypercall(handle.as_raw_fd(), &mut call).unwrap_or(-1);
                if result == 0 {
                    return Ok(version);
                }
            }
        }
        Err(Error::XenVersionUnsupported)
    }

    pub async fn mmap(&self, addr: u64, len: u64) -> Option<u64> {
        let _permit = self.semaphore.acquire().await.ok()?;
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
                trace!(
                    "call fd={} mmap addr={:#x} len={} = {:#x}",
                    self.handle.as_raw_fd(),
                    addr,
                    len,
                    ptr as u64,
                );
                Some(ptr as u64)
            }
        }
    }

    pub async fn hypercall(&self, op: c_ulong, arg: [c_ulong; 5]) -> Result<c_long> {
        let _permit = self.semaphore.acquire().await?;
        trace!(
            "call fd={} hypercall op={:#x} arg={:?}",
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

    pub async fn hypercall0(&self, op: c_ulong) -> Result<c_long> {
        self.hypercall(op, [0, 0, 0, 0, 0]).await
    }

    pub async fn hypercall1(&self, op: c_ulong, arg1: c_ulong) -> Result<c_long> {
        self.hypercall(op, [arg1, 0, 0, 0, 0]).await
    }

    pub async fn hypercall2(&self, op: c_ulong, arg1: c_ulong, arg2: c_ulong) -> Result<c_long> {
        self.hypercall(op, [arg1, arg2, 0, 0, 0]).await
    }

    pub async fn hypercall3(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
    ) -> Result<c_long> {
        self.hypercall(op, [arg1, arg2, arg3, 0, 0]).await
    }

    pub async fn hypercall4(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
    ) -> Result<c_long> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, 0]).await
    }

    pub async fn hypercall5(
        &self,
        op: c_ulong,
        arg1: c_ulong,
        arg2: c_ulong,
        arg3: c_ulong,
        arg4: c_ulong,
        arg5: c_ulong,
    ) -> Result<c_long> {
        self.hypercall(op, [arg1, arg2, arg3, arg4, arg5]).await
    }

    pub async fn multicall(&self, calls: &mut [MultiCallEntry]) -> Result<()> {
        trace!(
            "call fd={} multicall calls={:?}",
            self.handle.as_raw_fd(),
            calls
        );
        self.hypercall2(
            HYPERVISOR_MULTICALL,
            calls.as_mut_ptr() as c_ulong,
            calls.len() as c_ulong,
        )
        .await?;
        Ok(())
    }

    pub async fn map_resource(
        &self,
        domid: u32,
        typ: u32,
        id: u32,
        idx: u32,
        num: u64,
        addr: u64,
    ) -> Result<()> {
        let _permit = self.semaphore.acquire().await?;
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

    pub async fn mmap_batch(
        &self,
        domid: u32,
        num: u64,
        addr: u64,
        mfns: Vec<u64>,
    ) -> Result<c_long> {
        let _permit = self.semaphore.acquire().await?;
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
                mfns: mfns.as_mut_ptr() as u64,
                errors: errors.as_mut_ptr() as u64,
            };

            let result = sys::mmapbatch(self.handle.as_raw_fd(), &mut batch);
            if let Err(errno) = result {
                if errno != Errno::ENOENT {
                    return Err(Error::MmapBatchFailed(errno))?;
                }

                sleep(Duration::from_micros(100)).await;

                let mut i: usize = 0;
                let mut paged: usize = 0;
                loop {
                    if errors[i] != libc::ENOENT {
                        i += 1;
                        continue;
                    }

                    paged += 1;
                    let mut batch = MmapBatch {
                        num: 1,
                        domid: domid as u16,
                        addr: addr + ((i as u64) << 12),
                        mfns: mfns.as_mut_ptr().add(i) as u64,
                        errors: errors.as_mut_ptr().add(i) as u64,
                    };

                    loop {
                        i += 1;
                        if i < num as usize {
                            if errors[i] != libc::ENOENT {
                                break;
                            }
                            batch.num += 1;
                        }
                    }

                    let result = sys::mmapbatch(self.handle.as_raw_fd(), &mut batch);
                    if let Err(n) = result {
                        if n != Errno::ENOENT {
                            return Err(Error::MmapBatchFailed(n))?;
                        }
                    }

                    if i < num as usize {
                        break;
                    }

                    let count = result.unwrap();
                    if count <= 0 {
                        break;
                    }
                }

                return Ok(paged as c_long);
            }
            Ok(result.unwrap() as c_long)
        }
    }

    pub async fn get_version_capabilities(&self) -> Result<XenCapabilitiesInfo> {
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
        )
        .await?;
        Ok(info)
    }

    pub async fn evtchn_op(&self, cmd: c_int, arg: u64) -> Result<()> {
        self.hypercall2(HYPERVISOR_EVENT_CHANNEL_OP, cmd as c_ulong, arg)
            .await?;
        Ok(())
    }

    pub async fn evtchn_alloc_unbound(&self, domid: u32, remote_domid: u32) -> Result<u32> {
        let mut alloc_unbound = EvtChnAllocUnbound {
            dom: domid as u16,
            remote_dom: remote_domid as u16,
            port: 0,
        };
        self.evtchn_op(6, addr_of_mut!(alloc_unbound) as c_ulong)
            .await?;
        Ok(alloc_unbound.port)
    }

    pub async fn get_domain_info(&self, domid: u32) -> Result<GetDomainInfo> {
        trace!(
            "domctl fd={} get_domain_info domid={}",
            self.handle.as_raw_fd(),
            domid
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_GETDOMAININFO,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                get_domain_info: GetDomainInfo::default(),
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(unsafe { domctl.value.get_domain_info })
    }

    pub async fn create_domain(&self, create_domain: CreateDomain) -> Result<u32> {
        trace!(
            "domctl fd={} create_domain create_domain={:?}",
            self.handle.as_raw_fd(),
            create_domain
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_CREATEDOMAIN,
            interface_version: self.domctl_interface_version,
            domid: 0,
            value: DomCtlValue { create_domain },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(domctl.domid)
    }

    pub async fn pause_domain(&self, domid: u32) -> Result<()> {
        trace!(
            "domctl fd={} pause_domain domid={:?}",
            self.handle.as_raw_fd(),
            domid,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_PAUSEDOMAIN,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn unpause_domain(&self, domid: u32) -> Result<()> {
        trace!(
            "domctl fd={} unpause_domain domid={:?}",
            self.handle.as_raw_fd(),
            domid,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_UNPAUSEDOMAIN,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn set_max_mem(&self, domid: u32, memkb: u64) -> Result<()> {
        trace!(
            "domctl fd={} set_max_mem domid={} memkb={}",
            self.handle.as_raw_fd(),
            domid,
            memkb
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_MAX_MEM,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                max_mem: MaxMem { max_memkb: memkb },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn set_max_vcpus(&self, domid: u32, max_vcpus: u32) -> Result<()> {
        trace!(
            "domctl fd={} set_max_vcpus domid={} max_vcpus={}",
            self.handle.as_raw_fd(),
            domid,
            max_vcpus
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_MAX_VCPUS,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                max_cpus: MaxVcpus { max_vcpus },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn set_address_size(&self, domid: u32, size: u32) -> Result<()> {
        trace!(
            "domctl fd={} set_address_size domid={} size={}",
            self.handle.as_raw_fd(),
            domid,
            size,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_SET_ADDRESS_SIZE,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                address_size: AddressSize { size },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn set_vcpu_context(
        &self,
        domid: u32,
        vcpu: u32,
        mut context: VcpuGuestContextAny,
    ) -> Result<()> {
        trace!(
            "domctl fd={} set_vcpu_context domid={} context={:?}",
            self.handle.as_raw_fd(),
            domid,
            unsafe { context.value }
        );

        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_SETVCPUCONTEXT,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                vcpu_context: DomCtlVcpuContext {
                    vcpu,
                    ctx: addr_of_mut!(context) as c_ulong,
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn get_page_frame_info(&self, domid: u32, frames: &[u64]) -> Result<Vec<u64>> {
        let mut buffer: Vec<u64> = frames.to_vec();
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_GETPAGEFRAMEINFO3,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                get_page_frame_info: GetPageFrameInfo3 {
                    num: buffer.len() as u64,
                    array: buffer.as_mut_ptr() as c_ulong,
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        let slice = unsafe {
            slice::from_raw_parts_mut(
                domctl.value.get_page_frame_info.array as *mut u64,
                domctl.value.get_page_frame_info.num as usize,
            )
        };
        Ok(slice.to_vec())
    }

    pub async fn hypercall_init(&self, domid: u32, gmfn: u64) -> Result<()> {
        trace!(
            "domctl fd={} hypercall_init domid={} gmfn={}",
            self.handle.as_raw_fd(),
            domid,
            gmfn
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_HYPERCALL_INIT,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                hypercall_init: HypercallInit { gmfn },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn destroy_domain(&self, domid: u32) -> Result<()> {
        trace!(
            "domctl fd={} destroy_domain domid={}",
            self.handle.as_raw_fd(),
            domid
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_DESTROYDOMAIN,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue { pad: [0; 128] },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn get_memory_map(&self, max_entries: u32) -> Result<Vec<E820Entry>> {
        let mut memory_map = MemoryMap {
            count: max_entries,
            buffer: 0,
        };
        let mut entries = vec![E820Entry::default(); max_entries as usize];
        memory_map.buffer = entries.as_mut_ptr() as c_ulong;
        self.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_MEMORY_MAP as c_ulong,
            addr_of_mut!(memory_map) as c_ulong,
        )
        .await?;
        entries.truncate(memory_map.count as usize);
        Ok(entries)
    }

    pub async fn set_memory_map(
        &self,
        domid: u32,
        entries: Vec<E820Entry>,
    ) -> Result<Vec<E820Entry>> {
        trace!(
            "fd={} set_memory_map domid={} entries={:?}",
            self.handle.as_raw_fd(),
            domid,
            entries
        );
        let mut memory_map = ForeignMemoryMap {
            domid: domid as u16,
            map: MemoryMap {
                count: entries.len() as u32,
                buffer: entries.as_ptr() as u64,
            },
        };
        self.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_SET_MEMORY_MAP as c_ulong,
            addr_of_mut!(memory_map) as c_ulong,
        )
        .await?;
        Ok(entries)
    }

    pub async fn populate_physmap(
        &self,
        domid: u32,
        nr_extents: u64,
        extent_order: u32,
        mem_flags: u32,
        extent_starts: &[u64],
    ) -> Result<Vec<u64>> {
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

        let code = self
            .hypercall2(
                HYPERVISOR_MEMORY_OP,
                XEN_MEM_POPULATE_PHYSMAP as c_ulong,
                addr_of_mut!(reservation) as c_ulong,
            )
            .await?;
        if code as usize != extent_starts.len() {
            return Err(Error::PopulatePhysmapFailed);
        }
        let extents = extent_starts[0..code as usize].to_vec();
        Ok(extents)
    }

    pub async fn claim_pages(&self, domid: u32, pages: u64) -> Result<()> {
        trace!(
            "memory fd={} claim_pages domid={} pages={}",
            self.handle.as_raw_fd(),
            domid,
            pages
        );
        let mut reservation = MemoryReservation {
            extent_start: 0,
            nr_extents: pages,
            extent_order: 0,
            mem_flags: 0,
            domid: domid as u16,
        };
        self.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_CLAIM_PAGES as c_ulong,
            addr_of_mut!(reservation) as c_ulong,
        )
        .await?;
        Ok(())
    }

    pub async fn mmuext(&self, domid: u32, cmd: c_uint, arg1: u64, arg2: u64) -> Result<()> {
        let mut ops = MmuExtOp { cmd, arg1, arg2 };

        self.hypercall4(
            HYPERVISOR_MMUEXT_OP,
            addr_of_mut!(ops) as c_ulong,
            1,
            0,
            domid as c_ulong,
        )
        .await
        .map(|_| ())
    }

    pub async fn iomem_permission(
        &self,
        domid: u32,
        first_mfn: u64,
        nr_mfns: u64,
        allow: bool,
    ) -> Result<()> {
        trace!(
            "domctl fd={} iomem_permission domid={} first_mfn={:#x}, nr_mfns={:#x} allow={}",
            self.handle.as_raw_fd(),
            domid,
            first_mfn,
            nr_mfns,
            allow,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_IOMEM_PERMISSION,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                iomem_permission: IoMemPermission {
                    first_mfn,
                    nr_mfns,
                    allow: if allow { 1 } else { 0 },
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn ioport_permission(
        &self,
        domid: u32,
        first_port: u32,
        nr_ports: u32,
        allow: bool,
    ) -> Result<()> {
        trace!(
            "domctl fd={} ioport_permission domid={} first_port={:#x}, nr_ports={:#x} allow={}",
            self.handle.as_raw_fd(),
            domid,
            first_port,
            nr_ports,
            allow,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_IOPORT_PERMISSION,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                ioport_permission: IoPortPermission {
                    first_port,
                    nr_ports,
                    allow: if allow { 1 } else { 0 },
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn irq_permission(&self, domid: u32, irq: u32, allow: bool) -> Result<()> {
        trace!(
            "domctl fd={} irq_permission domid={} irq={} allow={}",
            self.handle.as_raw_fd(),
            domid,
            irq,
            allow,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_IRQ_PERMISSION,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                irq_permission: IrqPermission {
                    pirq: irq,
                    allow: if allow { 1 } else { 0 },
                    pad: [0; 3],
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    #[allow(clippy::field_reassign_with_default)]
    pub async fn map_pirq(&self, domid: u32, index: isize, pirq: Option<u32>) -> Result<u32> {
        trace!(
            "physdev fd={} map_pirq domid={} index={} pirq={:?}",
            self.handle.as_raw_fd(),
            domid,
            index,
            pirq,
        );
        let mut physdev = PhysdevMapPirq {
            domid: domid as u16,
            typ: 0x1,
            index: index as c_int,
            pirq: pirq.map(|x| x as c_int).unwrap_or(index as c_int),
            ..Default::default()
        };
        physdev.domid = domid as u16;
        physdev.typ = 0x1;
        physdev.index = index as c_int;
        physdev.pirq = pirq.map(|x| x as c_int).unwrap_or(index as c_int);
        self.hypercall2(
            HYPERVISOR_PHYSDEV_OP,
            PHYSDEVOP_MAP_PIRQ,
            addr_of_mut!(physdev) as c_ulong,
        )
        .await?;
        Ok(physdev.pirq as u32)
    }

    pub async fn assign_device(&self, domid: u32, sbdf: u32, flags: u32) -> Result<()> {
        trace!(
            "domctl fd={} assign_device domid={} sbdf={} flags={}",
            self.handle.as_raw_fd(),
            domid,
            sbdf,
            flags,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_ASSIGN_DEVICE,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                assign_device: AssignDevice {
                    device: DOMCTL_DEV_PCI,
                    flags,
                    pci_assign_device: PciAssignDevice { sbdf, padding: 0 },
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }

    #[allow(clippy::field_reassign_with_default)]
    pub async fn set_hvm_param(&self, domid: u32, index: u32, value: u64) -> Result<()> {
        trace!(
            "set_hvm_param fd={} domid={} index={} value={:?}",
            self.handle.as_raw_fd(),
            domid,
            index,
            value,
        );
        let mut param = HvmParam::default();
        param.domid = domid as u16;
        param.index = index;
        param.value = value;
        self.hypercall2(HYPERVISOR_HVM_OP, 0, addr_of_mut!(param) as c_ulong)
            .await?;
        Ok(())
    }

    pub async fn get_hvm_context(&self, domid: u32, buffer: Option<&mut [u8]>) -> Result<u32> {
        trace!(
            "domctl fd={} get_hvm_context domid={}",
            self.handle.as_raw_fd(),
            domid,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_GETHVMCONTEXT,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                hvm_context: HvmContext {
                    size: buffer.as_ref().map(|x| x.len()).unwrap_or(0) as u32,
                    buffer: buffer.map(|x| x.as_mut_ptr()).unwrap_or(null_mut()) as u64,
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(unsafe { domctl.value.hvm_context.size })
    }

    pub async fn set_hvm_context(&self, domid: u32, buffer: &mut [u8]) -> Result<u32> {
        trace!(
            "domctl fd={} set_hvm_context domid={}",
            self.handle.as_raw_fd(),
            domid,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_SETHVMCONTEXT,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                hvm_context: HvmContext {
                    size: buffer.len() as u32,
                    buffer: buffer.as_ptr() as u64,
                },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(unsafe { domctl.value.hvm_context.size })
    }

    pub async fn set_paging_mempool_size(&self, domid: u32, size: u64) -> Result<()> {
        trace!(
            "domctl fd={} set_paging_mempool_size domid={} size={}",
            self.handle.as_raw_fd(),
            domid,
            size,
        );
        let mut domctl = DomCtl {
            cmd: XEN_DOMCTL_SET_PAGING_MEMPOOL_SIZE,
            interface_version: self.domctl_interface_version,
            domid,
            value: DomCtlValue {
                paging_mempool: PagingMempool { size },
            },
        };
        self.hypercall1(HYPERVISOR_DOMCTL, addr_of_mut!(domctl) as c_ulong)
            .await?;
        Ok(())
    }
}
