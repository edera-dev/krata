use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub enum LaunchPackedFormat {
    Squashfs,
    Erofs,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LaunchNetworkIpv4 {
    pub address: String,
    pub gateway: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LaunchNetworkIpv6 {
    pub address: String,
    pub gateway: String,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LaunchNetworkResolver {
    pub nameservers: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LaunchNetwork {
    pub link: String,
    pub ipv4: LaunchNetworkIpv4,
    pub ipv6: LaunchNetworkIpv6,
    pub resolver: LaunchNetworkResolver,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LaunchRoot {
    pub format: LaunchPackedFormat,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LaunchInfo {
    pub root: LaunchRoot,
    pub hostname: Option<String>,
    pub network: Option<LaunchNetwork>,
    pub env: HashMap<String, String>,
    pub run: Option<Vec<String>>,
}
