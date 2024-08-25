use anyhow::Result;
use krata::v1::common::Zone;
use krata::v1::control::{ResolveZoneIdReply, ResolveZoneIdRequest};

use crate::db::zone::ZoneStore;

pub struct ResolveZoneIdRpc {
    zones: ZoneStore,
}

impl ResolveZoneIdRpc {
    pub fn new(zones: ZoneStore) -> Self {
        Self { zones }
    }

    pub async fn process(self, request: ResolveZoneIdRequest) -> Result<ResolveZoneIdReply> {
        let zones = self.zones.list().await?;
        let zones = zones
            .into_values()
            .filter(|x| {
                let comparison_spec = x.spec.as_ref().cloned().unwrap_or_default();
                (!request.name.is_empty() && comparison_spec.name == request.name)
                    || x.id == request.name
            })
            .collect::<Vec<Zone>>();
        Ok(ResolveZoneIdReply {
            zone_id: zones.first().cloned().map(|x| x.id).unwrap_or_default(),
        })
    }
}
