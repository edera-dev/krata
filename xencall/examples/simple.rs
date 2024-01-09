use std::ffi::c_ulong;
use std::ptr::addr_of;
use xencall::{XenCall, XenCallError};

fn main() -> Result<(), XenCallError> {
    let mut call = XenCall::open()?;
    let message = "Hello World";
    let bytes = message.as_bytes();
    call.hypercall3(18, 0, bytes.len() as c_ulong, addr_of!(bytes) as c_ulong)?;
    Ok(())
}
