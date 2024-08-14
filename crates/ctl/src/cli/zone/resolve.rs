use anyhow::Result;
use clap::Parser;
use krata::v1::control::{control_service_client::ControlServiceClient, ResolveZoneIdRequest};

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
            .resolve_zone_id(Request::new(ResolveZoneIdRequest {
                name: self.zone.clone(),
            }))
            .await?
            .into_inner();
        if !reply.zone_id.is_empty() {
            println!("{}", reply.zone_id);
        } else {
            std::process::exit(1);
        }
        Ok(())
    }
}
