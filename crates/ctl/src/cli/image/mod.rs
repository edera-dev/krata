use anyhow::Result;
use clap::{Parser, Subcommand};
use tonic::transport::Channel;

use krata::events::EventStream;
use krata::v1::control::control_service_client::ControlServiceClient;

use crate::cli::image::pull::ImagePullCommand;

pub mod pull;

#[derive(Parser)]
#[command(about = "Manage the images on the isolation engine")]
pub struct ImageCommand {
    #[command(subcommand)]
    subcommand: ImageCommands,
}

impl ImageCommand {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        self.subcommand.run(client, events).await
    }
}

#[derive(Subcommand)]
pub enum ImageCommands {
    Pull(ImagePullCommand),
}

impl ImageCommands {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        match self {
            ImageCommands::Pull(pull) => pull.run(client).await,
        }
    }
}
