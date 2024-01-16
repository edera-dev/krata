use crate::sys::{
    MemoryMap, MemoryReservation, MmuExtOp, MultiCallEntry, HYPERVISOR_MEMORY_OP,
    HYPERVISOR_MMUEXT_OP, XEN_MEM_MEMORY_MAP, XEN_MEM_POPULATE_PHYSMAP,
};
use crate::{XenCall, XenCallError};

use log::trace;
use std::ffi::{c_uint, c_ulong};
use std::os::fd::AsRawFd;
use std::ptr::addr_of_mut;

pub struct MemoryControl<'a> {
    call: &'a XenCall,
}

impl MemoryControl<'_> {
    pub fn new(call: &XenCall) -> MemoryControl {
        MemoryControl { call }
    }

    pub fn get_memory_map(&self, size_of_entry: usize) -> Result<Vec<u8>, XenCallError> {
        let mut memory_map = MemoryMap {
            count: 0,
            buffer: 0,
        };
        self.call.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_MEMORY_MAP as c_ulong,
            addr_of_mut!(memory_map) as c_ulong,
        )?;
        let mut buffer = vec![0u8; memory_map.count as usize * size_of_entry];
        memory_map.buffer = buffer.as_mut_ptr() as c_ulong;
        self.call.hypercall2(
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
        trace!("memory fd={} populate_physmap domid={} nr_extents={} extent_order={} mem_flags={} extent_starts={:?}", self.call.handle.as_raw_fd(), domid, nr_extents, extent_order, mem_flags, extent_starts);
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
        self.call.multicall(calls)?;
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

        self.call
            .hypercall4(
                HYPERVISOR_MMUEXT_OP,
                addr_of_mut!(ops) as c_ulong,
                1,
                0,
                domid as c_ulong,
            )
            .map(|_| ())
    }
}
