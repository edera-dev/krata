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
    #[arg(
        short = 'C',
        long = "max-cpus",
        default_value_t = 0,
        help = "Maximum vCPUs available to the zone (0 means previous value)"
    )]
    max_cpus: u32,
    #[arg(
        short = 'c',
        long = "target-cpus",
        default_value_t = 0,
        help = "Target vCPUs for the zone to use (0 means previous value)"
    )]
    target_cpus: u32,
    #[arg(
        short = 'M',
        long = "max-memory",
        default_value_t = 0,
        help = "Maximum memory available to the zone, in megabytes (0 means previous value)"
    )]
    max_memory: u64,
    #[arg(
        short = 'm',
        long = "target-memory",
        default_value_t = 0,
        help = "Target memory for the zone to use, in megabytes (0 means previous value)"
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
                    max_cpus: if self.max_cpus == 0 {
                        active_resources.max_cpus
                    } else {
                        self.max_cpus
                    },
                    target_cpus: if self.target_cpus == 0 {
                        active_resources.target_cpus
                    } else {
                        self.target_cpus
                    },
                }),
            }))
            .await?;
        Ok(())
    }
}
