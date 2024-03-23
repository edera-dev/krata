use anyhow::Result;
use clap::Parser;
use krata::control::{control_service_client::ControlServiceClient, DestroyGuestRequest};

use tonic::{transport::Channel, Request};

use crate::cli::resolve_guest;

#[derive(Parser)]
pub struct DestroyCommand {
    #[arg()]
    guest: String,
}

impl DestroyCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let guest_id: String = resolve_guest(&mut client, &self.guest).await?;
        let _ = client
            .destroy_guest(Request::new(DestroyGuestRequest { guest_id }))
            .await?
            .into_inner();
        println!("destroyed guest: {}", self.guest);
        Ok(())
    }
}
