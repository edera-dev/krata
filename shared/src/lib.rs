use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchNetworkIpv4 {
    pub address: String,
    pub gateway: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchNetwork {
    pub link: String,
    pub ipv4: LaunchNetworkIpv4,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchInfo {
    pub network: Option<LaunchNetwork>,
    pub env: Option<Vec<String>>,
    pub run: Option<Vec<String>>,
}
