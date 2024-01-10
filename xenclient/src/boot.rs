use crate::XenClientError;

pub trait BootImageLoader {
    fn load(&self, dst: *mut u8) -> Result<BootImageInfo, XenClientError>;
}

#[derive(Debug)]
pub struct BootImageInfo {
    pub virt_kstart: u64,
    pub virt_kend: u64,
    pub entry: u64,
    pub hv_start_low: u64,
}
