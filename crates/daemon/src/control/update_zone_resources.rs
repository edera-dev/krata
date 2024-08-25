use std::str::FromStr;

use anyhow::{anyhow, Result};
use uuid::Uuid;

use krata::v1::common::{ZoneResourceStatus, ZoneState};
use krata::v1::control::{UpdateZoneResourcesReply, UpdateZoneResourcesRequest};
use kratart::Runtime;

use crate::db::zone::ZoneStore;

pub struct UpdateZoneResourcesRpc {
    runtime: Runtime,
    zones: ZoneStore,
}

impl UpdateZoneResourcesRpc {
    pub fn new(runtime: Runtime, zones: ZoneStore) -> Self {
        Self { runtime, zones }
    }

    pub async fn process(
        self,
        request: UpdateZoneResourcesRequest,
    ) -> Result<UpdateZoneResourcesReply> {
        let uuid = Uuid::from_str(&request.zone_id)?;
        let Some(mut zone) = self.zones.read(uuid).await? else {
            return Err(anyhow!("zone not found"));
        };

        let Some(ref mut status) = zone.status else {
            return Err(anyhow!("zone state not available"));
        };

        if status.state() != ZoneState::Created {
            return Err(anyhow!("zone is in an invalid state"));
        }

        if status.domid == 0 || status.domid == u32::MAX {
            return Err(anyhow!("zone domid is invalid"));
        }

        let mut resources = request.resources.unwrap_or_default();
        if resources.target_memory > resources.max_memory {
            resources.max_memory = resources.target_memory;
        }

        if resources.target_cpus < 1 {
            resources.target_cpus = 1;
        }

        let initial_resources = zone
            .spec
            .clone()
            .unwrap_or_default()
            .initial_resources
            .unwrap_or_default();
        if resources.target_cpus > initial_resources.max_cpus {
            resources.target_cpus = initial_resources.max_cpus;
        }
        resources.max_cpus = initial_resources.max_cpus;

        self.runtime
            .set_memory_resources(
                status.domid,
                resources.target_memory * 1024 * 1024,
                resources.max_memory * 1024 * 1024,
            )
            .await
            .map_err(|error| anyhow!("failed to set memory resources: {}", error))?;
        self.runtime
            .set_cpu_resources(status.domid, resources.target_cpus)
            .await
            .map_err(|error| anyhow!("failed to set cpu resources: {}", error))?;
        status.resource_status = Some(ZoneResourceStatus {
            active_resources: Some(resources),
        });

        self.zones.update(uuid, zone).await?;
        Ok(UpdateZoneResourcesReply {})
    }
}
