use anyhow::Result;
use clap::{Parser, Subcommand};
use reservation::NetworkReservationCommand;
use tonic::transport::Channel;

use krata::events::EventStream;
use krata::v1::control::control_service_client::ControlServiceClient;

pub mod reservation;

#[derive(Parser)]
#[command(about = "Manage the network on the isolation engine")]
pub struct NetworkCommand {
    #[command(subcommand)]
    subcommand: NetworkCommands,
}

impl NetworkCommand {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        self.subcommand.run(client, events).await
    }
}

#[derive(Subcommand)]
pub enum NetworkCommands {
    Reservation(NetworkReservationCommand),
}

impl NetworkCommands {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        match self {
            NetworkCommands::Reservation(reservation) => reservation.run(client, events).await,
        }
    }
}
