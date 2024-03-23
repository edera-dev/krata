use once_cell::sync::Lazy;
use prost_reflect::DescriptorPool;

pub mod common;
pub mod control;
pub mod dial;
pub mod launchcfg;

#[cfg(target_os = "linux")]
pub mod ethtool;

pub static DESCRIPTOR_POOL: Lazy<DescriptorPool> = Lazy::new(|| {
    DescriptorPool::decode(
        include_bytes!(concat!(env!("OUT_DIR"), "/file_descriptor_set.bin")).as_ref(),
    )
    .unwrap()
});
