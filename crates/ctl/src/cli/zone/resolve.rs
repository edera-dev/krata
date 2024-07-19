use anyhow::Result;
use clap::Parser;
use krata::v1::control::{control_service_client::ControlServiceClient, ResolveZoneRequest};

use tonic::{transport::Channel, Request};

#[derive(Parser)]
#[command(about = "Resolve a zone name to a uuid")]
pub struct ZoneResolveCommand {
    #[arg(help = "Zone name")]
    zone: String,
}

impl ZoneResolveCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let reply = client
            .resolve_zone(Request::new(ResolveZoneRequest {
                name: self.zone.clone(),
            }))
            .await?
            .into_inner();
        if let Some(zone) = reply.zone {
            println!("{}", zone.id);
        } else {
            std::process::exit(1);
        }
        Ok(())
    }
}
