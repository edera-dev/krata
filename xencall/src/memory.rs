use crate::sys::{MemoryReservation, HYPERVISOR_MEMORY_OP, XEN_MEM_POPULATE_PHYSMAP};
use crate::{XenCall, XenCallError};

use std::ffi::c_ulong;

use std::ptr::addr_of_mut;

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
        let mut extent_starts = extent_starts.to_vec();
        let mut reservation = MemoryReservation {
            extent_start: extent_starts.as_mut_ptr() as c_ulong,
            nr_extents,
            extent_order,
            mem_flags,
            domid: domid as u16,
        };
        let code = self.call.hypercall2(
            HYPERVISOR_MEMORY_OP,
            XEN_MEM_POPULATE_PHYSMAP as c_ulong,
            addr_of_mut!(reservation) as c_ulong,
        )?;

        if code < 0 {
            return Err(XenCallError::new("failed to populate physmap"));
        }

        if code as usize > extent_starts.len() {
            return Err(XenCallError::new("failed to populate physmap"));
        }

        Ok(extent_starts[0..code as usize].to_vec())
    }
}
