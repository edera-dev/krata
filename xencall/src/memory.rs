use crate::sys::{MemoryReservation, HYPERVISOR_MEMORY_OP, XEN_MEM_POPULATE_PHYSMAP};
use crate::{XenCall, XenCallError};

use std::ffi::c_ulong;

use std::ptr::addr_of;

pub struct MemoryControl<'a> {
    call: &'a XenCall,
}

impl MemoryControl<'_> {
    pub fn new(call: &XenCall) -> MemoryControl {
        MemoryControl { call }
    }

    pub fn populate_physmap(
        &self,
        domid: u32,
        nr_extents: u64,
        extent_order: u32,
        mem_flags: u32,
        extent_starts: &[u64],
    ) -> Result<Vec<u64>, XenCallError> {
        let extent_starts = extent_starts.to_vec();
        let reservation = MemoryReservation {
            extent_start: addr_of!(extent_starts) as c_ulong,
            nr_extents,
            extent_order,
            mem_flags,
            domid: domid as u16,
        };
        self.call.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_POPULATE_PHYSMAP as c_ulong,
            addr_of!(reservation) as c_ulong,
        )?;
        Ok(extent_starts)
    }
}
