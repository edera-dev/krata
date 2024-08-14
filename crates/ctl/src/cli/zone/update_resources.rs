use anyhow::Result;
use clap::Parser;
use krata::v1::{
    common::ZoneResourceSpec,
    control::{control_service_client::ControlServiceClient, UpdateZoneResourcesRequest},
};

use crate::cli::resolve_zone;
use krata::v1::control::GetZoneRequest;
use tonic::{transport::Channel, Request};

#[derive(Parser)]
#[command(about = "Update the available resources to a zone")]
pub struct ZoneUpdateResourcesCommand {
    #[arg(help = "Zone to update resources of, either the name or the uuid")]
    zone: String,
    #[arg(short, long, default_value_t = 0, help = "vCPUs available to the zone")]
    cpus: u32,
    #[arg(
        short = 'M',
        long = "max-memory",
        default_value_t = 0,
        help = "Maximum memory available to the zone, in megabytes"
    )]
    max_memory: u64,
    #[arg(
        short = 'm',
        long = "target-memory",
        default_value_t = 0,
        help = "Memory target for the zone, in megabytes"
    )]
    target_memory: u64,
}

impl ZoneUpdateResourcesCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let zone_id = resolve_zone(&mut client, &self.zone).await?;
        let zone = client
            .get_zone(GetZoneRequest { zone_id })
            .await?
            .into_inner()
            .zone
            .unwrap_or_default();
        let active_resources = zone
            .status
            .clone()
            .unwrap_or_default()
            .resource_status
            .unwrap_or_default()
            .active_resources
            .unwrap_or_default();
        client
            .update_zone_resources(Request::new(UpdateZoneResourcesRequest {
                zone_id: zone.id.clone(),
                resources: Some(ZoneResourceSpec {
                    max_memory: if self.max_memory == 0 {
                        active_resources.max_memory
                    } else {
                        self.max_memory
                    },
                    target_memory: if self.target_memory == 0 {
                        active_resources.target_memory
                    } else {
                        self.target_memory
                    },
                    cpus: if self.cpus == 0 {
                        active_resources.cpus
                    } else {
                        self.cpus
                    },
                }),
            }))
            .await?;
        Ok(())
    }
}
