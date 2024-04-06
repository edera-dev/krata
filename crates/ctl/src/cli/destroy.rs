use anyhow::Result;
use clap::Parser;
use krata::{
    events::EventStream,
    v1::{
        common::GuestStatus,
        control::{
            control_service_client::ControlServiceClient, watch_events_reply::Event,
            DestroyGuestRequest,
        },
    },
};

use log::error;
use tonic::{transport::Channel, Request};

use crate::cli::resolve_guest;

#[derive(Parser)]
#[command(about = "Destroy a guest")]
pub struct DestroyCommand {
    #[arg(
        short = 'W',
        long,
        help = "Wait for the destruction of the guest to complete"
    )]
    wait: bool,
    #[arg(help = "Guest to destroy, either the name or the uuid")]
    guest: String,
}

impl DestroyCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let guest_id: String = resolve_guest(&mut client, &self.guest).await?;
        let _ = client
            .destroy_guest(Request::new(DestroyGuestRequest {
                guest_id: guest_id.clone(),
            }))
            .await?
            .into_inner();
        if self.wait {
            wait_guest_destroyed(&guest_id, events).await?;
        }
        Ok(())
    }
}

async fn wait_guest_destroyed(id: &str, events: EventStream) -> Result<()> {
    let mut stream = events.subscribe();
    while let Ok(event) = stream.recv().await {
        match event {
            Event::GuestChanged(changed) => {
                let Some(guest) = changed.guest else {
                    continue;
                };

                if guest.id != id {
                    continue;
                }

                let Some(state) = guest.state else {
                    continue;
                };

                if let Some(ref error) = state.error_info {
                    if state.status() == GuestStatus::Failed {
                        error!("destroy failed: {}", error.message);
                        std::process::exit(1);
                    } else {
                        error!("guest error: {}", error.message);
                    }
                }

                if state.status() == GuestStatus::Destroyed {
                    std::process::exit(0);
                }
            }
        }
    }
    Ok(())
}
