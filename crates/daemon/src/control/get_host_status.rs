use crate::command::DaemonCommand;
use crate::network::assignment::NetworkAssignment;
use crate::zlt::ZoneLookupTable;
use anyhow::Result;
use krata::v1::control::{GetHostStatusReply, GetHostStatusRequest};

pub struct GetHostStatusRpc {
    network: NetworkAssignment,
    zlt: ZoneLookupTable,
}

impl GetHostStatusRpc {
    pub fn new(ip: NetworkAssignment, zlt: ZoneLookupTable) -> Self {
        Self { network: ip, zlt }
    }

    pub async fn process(self, _request: GetHostStatusRequest) -> Result<GetHostStatusReply> {
        let host_reservation = self.network.retrieve(self.zlt.host_uuid()).await?;
        Ok(GetHostStatusReply {
            host_domid: self.zlt.host_domid(),
            host_uuid: self.zlt.host_uuid().to_string(),
            krata_version: DaemonCommand::version(),
            host_ipv4: host_reservation
                .as_ref()
                .map(|x| format!("{}/{}", x.ipv4, x.ipv4_prefix))
                .unwrap_or_default(),
            host_ipv6: host_reservation
                .as_ref()
                .map(|x| format!("{}/{}", x.ipv6, x.ipv6_prefix))
                .unwrap_or_default(),
            host_mac: host_reservation
                .as_ref()
                .map(|x| x.mac.to_string().to_lowercase().replace('-', ":"))
                .unwrap_or_default(),
        })
    }
}
