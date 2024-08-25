use std::str::FromStr;

use anyhow::{anyhow, Result};
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

use krata::v1::common::ZoneState;
use krata::v1::control::{DestroyZoneReply, DestroyZoneRequest};

use crate::db::zone::ZoneStore;

pub struct DestroyZoneRpc {
    zones: ZoneStore,
    zone_reconciler_notify: Sender<Uuid>,
}

impl DestroyZoneRpc {
    pub fn new(zones: ZoneStore, zone_reconciler_notify: Sender<Uuid>) -> Self {
        Self {
            zones,
            zone_reconciler_notify,
        }
    }

    pub async fn process(self, request: DestroyZoneRequest) -> Result<DestroyZoneReply> {
        let uuid = Uuid::from_str(&request.zone_id)?;
        let Some(mut zone) = self.zones.read(uuid).await? else {
            return Err(anyhow!("zone not found"));
        };

        zone.status = Some(zone.status.as_mut().cloned().unwrap_or_default());

        if zone.status.as_ref().unwrap().state() == ZoneState::Destroyed {
            return Err(anyhow!("zone already destroyed"));
        }

        zone.status.as_mut().unwrap().state = ZoneState::Destroying.into();
        self.zones.update(uuid, zone).await?;
        self.zone_reconciler_notify.send(uuid).await?;
        Ok(DestroyZoneReply {})
    }
}
