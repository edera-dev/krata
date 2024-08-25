use std::str::FromStr;

use anyhow::Result;
use uuid::Uuid;

use krata::v1::control::{GetZoneReply, GetZoneRequest};

use crate::db::zone::ZoneStore;

pub struct GetZoneRpc {
    zones: ZoneStore,
}

impl GetZoneRpc {
    pub fn new(zones: ZoneStore) -> Self {
        Self { zones }
    }

    pub async fn process(self, request: GetZoneRequest) -> Result<GetZoneReply> {
        let mut zones = self.zones.list().await?;
        let zone = zones.remove(&Uuid::from_str(&request.zone_id)?);
        Ok(GetZoneReply { zone })
    }
}
