use anyhow::Result;
use clap::Parser;
use krata::control::{control_service_client::ControlServiceClient, ResolveGuestRequest};

use tonic::{transport::Channel, Request};

#[derive(Parser)]
pub struct ResolveCommand {
    #[arg()]
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
