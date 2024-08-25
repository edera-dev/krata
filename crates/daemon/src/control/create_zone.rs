use crate::db::zone::ZoneStore;
use crate::zlt::ZoneLookupTable;
use anyhow::{anyhow, Result};
use krata::v1::common::{Zone, ZoneState, ZoneStatus};
use krata::v1::control::{CreateZoneReply, CreateZoneRequest};
use tokio::sync::mpsc::Sender;
use uuid::Uuid;

pub struct CreateZoneRpc {
    zones: ZoneStore,
    zlt: ZoneLookupTable,
    zone_reconciler_notify: Sender<Uuid>,
}

impl CreateZoneRpc {
    pub fn new(
        zones: ZoneStore,
        zlt: ZoneLookupTable,
        zone_reconciler_notify: Sender<Uuid>,
    ) -> Self {
        Self {
            zones,
            zlt,
            zone_reconciler_notify,
        }
    }

    pub async fn process(self, request: CreateZoneRequest) -> Result<CreateZoneReply> {
        let Some(spec) = request.spec else {
            return Err(anyhow!("zone spec not provided"));
        };
        let uuid = Uuid::new_v4();
        self.zones
            .update(
                uuid,
                Zone {
                    id: uuid.to_string(),
                    status: Some(ZoneStatus {
                        state: ZoneState::Creating.into(),
                        network_status: None,
                        exit_status: None,
                        error_status: None,
                        resource_status: None,
                        host: self.zlt.host_uuid().to_string(),
                        domid: u32::MAX,
                    }),
                    spec: Some(spec),
                },
            )
            .await?;
        self.zone_reconciler_notify.send(uuid).await?;
        Ok(CreateZoneReply {
            zone_id: uuid.to_string(),
        })
    }
}
