use anyhow::Result;
use clap::{Parser, Subcommand};
use tonic::transport::Channel;

use krata::events::EventStream;
use krata::v1::control::control_service_client::ControlServiceClient;

use crate::cli::device::list::DeviceListCommand;

pub mod list;

#[derive(Parser)]
#[command(about = "Manage the devices on the isolation engine")]
pub struct DeviceCommand {
    #[command(subcommand)]
    subcommand: DeviceCommands,
}

impl DeviceCommand {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        self.subcommand.run(client, events).await
    }
}

#[derive(Subcommand)]
pub enum DeviceCommands {
    List(DeviceListCommand),
}

impl DeviceCommands {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        match self {
            DeviceCommands::List(list) => list.run(client, events).await,
        }
    }
}
