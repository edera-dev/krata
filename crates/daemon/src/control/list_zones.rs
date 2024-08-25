use anyhow::Result;
use krata::v1::common::Zone;
use krata::v1::control::{ListZonesReply, ListZonesRequest};

use crate::db::zone::ZoneStore;

pub struct ListZonesRpc {
    zones: ZoneStore,
}

impl ListZonesRpc {
    pub fn new(zones: ZoneStore) -> Self {
        Self { zones }
    }

    pub async fn process(self, _request: ListZonesRequest) -> Result<ListZonesReply> {
        let zones = self.zones.list().await?;
        let zones = zones.into_values().collect::<Vec<Zone>>();
        Ok(ListZonesReply { zones })
    }
}
