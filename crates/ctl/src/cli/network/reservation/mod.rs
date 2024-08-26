use anyhow::Result;
use clap::{Parser, Subcommand};
use list::NetworkReservationListCommand;
use tonic::transport::Channel;

use krata::events::EventStream;
use krata::v1::control::control_service_client::ControlServiceClient;

pub mod list;

#[derive(Parser)]
#[command(about = "Manage network reservations")]
pub struct NetworkReservationCommand {
    #[command(subcommand)]
    subcommand: NetworkReservationCommands,
}

impl NetworkReservationCommand {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        self.subcommand.run(client, events).await
    }
}

#[derive(Subcommand)]
pub enum NetworkReservationCommands {
    List(NetworkReservationListCommand),
}

impl NetworkReservationCommands {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        match self {
            NetworkReservationCommands::List(list) => list.run(client, events).await,
        }
    }
}
