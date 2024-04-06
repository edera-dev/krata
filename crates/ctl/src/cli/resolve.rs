use anyhow::Result;
use clap::Parser;
use krata::v1::control::{control_service_client::ControlServiceClient, ResolveGuestRequest};

use tonic::{transport::Channel, Request};

#[derive(Parser)]
#[command(about = "Resolve a guest name to a uuid")]
pub struct ResolveCommand {
    #[arg(help = "Guest name")]
    guest: String,
}

impl ResolveCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let reply = client
            .resolve_guest(Request::new(ResolveGuestRequest {
                name: self.guest.clone(),
            }))
            .await?
            .into_inner();
        if let Some(guest) = reply.guest {
            println!("{}", guest.id);
        } else {
            std::process::exit(1);
        }
        Ok(())
    }
}
