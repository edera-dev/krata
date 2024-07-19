use anyhow::Result;
use clap::Parser;
use krata::{
    events::EventStream,
    v1::{
        common::ZoneStatus,
        control::{
            control_service_client::ControlServiceClient, watch_events_reply::Event,
            DestroyZoneRequest,
        },
    },
};

use log::error;
use tonic::{transport::Channel, Request};

use crate::cli::resolve_zone;

#[derive(Parser)]
#[command(about = "Destroy a zone")]
pub struct ZoneDestroyCommand {
    #[arg(
        short = 'W',
        long,
        help = "Wait for the destruction of the zone to complete"
    )]
    wait: bool,
    #[arg(help = "Zone to destroy, either the name or the uuid")]
    zone: String,
}

impl ZoneDestroyCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let zone_id: String = resolve_zone(&mut client, &self.zone).await?;
        let _ = client
            .destroy_zone(Request::new(DestroyZoneRequest {
                zone_id: zone_id.clone(),
            }))
            .await?
            .into_inner();
        if self.wait {
            wait_zone_destroyed(&zone_id, events).await?;
        }
        Ok(())
    }
}

async fn wait_zone_destroyed(id: &str, events: EventStream) -> Result<()> {
    let mut stream = events.subscribe();
    while let Ok(event) = stream.recv().await {
        let Event::ZoneChanged(changed) = event;
        let Some(zone) = changed.zone else {
            continue;
        };

        if zone.id != id {
            continue;
        }

        let Some(state) = zone.state else {
            continue;
        };

        if let Some(ref error) = state.error_info {
            if state.status() == ZoneStatus::Failed {
                error!("destroy failed: {}", error.message);
                std::process::exit(1);
            } else {
                error!("zone error: {}", error.message);
            }
        }

        if state.status() == ZoneStatus::Destroyed {
            std::process::exit(0);
        }
    }
    Ok(())
}
