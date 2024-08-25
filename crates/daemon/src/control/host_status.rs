use crate::command::DaemonCommand;
use crate::ip::assignment::IpAssignment;
use crate::zlt::ZoneLookupTable;
use anyhow::Result;
use krata::v1::control::{HostStatusReply, HostStatusRequest};

pub struct HostStatusRpc {
    ip: IpAssignment,
    zlt: ZoneLookupTable,
}

impl HostStatusRpc {
    pub fn new(ip: IpAssignment, zlt: ZoneLookupTable) -> Self {
        Self { ip, zlt }
    }

    pub async fn process(self, _request: HostStatusRequest) -> Result<HostStatusReply> {
        let host_reservation = self.ip.retrieve(self.zlt.host_uuid()).await?;
        Ok(HostStatusReply {
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
