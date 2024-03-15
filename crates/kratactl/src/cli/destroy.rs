use anyhow::Result;
use clap::Parser;
use krata::control::{control_service_client::ControlServiceClient, DestroyGuestRequest};

use tonic::{transport::Channel, Request};

use crate::events::EventStream;

#[derive(Parser)]
pub struct DestroyCommand {
    #[arg()]
    guest: String,
}

impl DestroyCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let _ = client
            .destroy_guest(Request::new(DestroyGuestRequest {
                guest_id: self.guest.clone(),
            }))
            .await?
            .into_inner();
        println!("destroyed guest: {}", self.guest);
        Ok(())
    }
}
