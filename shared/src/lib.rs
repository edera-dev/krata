use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchNetworkIpv4 {
    pub address: String,
    pub gateway: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchNetworkIpv6 {
    pub address: String,
    pub gateway: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchNetworkResolver {
    pub nameservers: Vec<String>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchNetwork {
    pub link: String,
    pub ipv4: LaunchNetworkIpv4,
    pub ipv6: LaunchNetworkIpv6,
    pub resolver: LaunchNetworkResolver,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchChannels {
    pub exit: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchInfo {
    pub network: Option<LaunchNetwork>,
    pub env: Option<Vec<String>>,
    pub run: Option<Vec<String>>,
    pub channels: LaunchChannels,
}
