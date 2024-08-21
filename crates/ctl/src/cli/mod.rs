pub mod device;
pub mod host;
pub mod image;
pub mod zone;

use crate::cli::device::DeviceCommand;
use crate::cli::host::HostCommand;
use crate::cli::image::ImageCommand;
use crate::cli::zone::ZoneCommand;
use anyhow::{anyhow, Result};
use clap::Parser;
use krata::{
    client::ControlClientProvider,
    events::EventStream,
    v1::control::{control_service_client::ControlServiceClient, ResolveZoneIdRequest},
};
use tonic::{transport::Channel, Request};

#[derive(Parser)]
#[command(version, about = "Control the krata isolation engine")]
pub struct ControlCommand {
    #[arg(
        short,
        long,
        help = "The connection URL to the krata isolation engine",
        default_value = "unix:///var/lib/krata/daemon.socket"
    )]
    connection: String,

    #[command(subcommand)]
    command: ControlCommands,
}

#[allow(clippy::large_enum_variant)]
#[derive(Parser)]
pub enum ControlCommands {
    Zone(ZoneCommand),
    Image(ImageCommand),
    Device(DeviceCommand),
    Host(HostCommand),
}

impl ControlCommand {
    pub async fn run(self) -> Result<()> {
        let client = ControlClientProvider::dial(self.connection.parse()?).await?;
        let events = EventStream::open(client.clone()).await?;
        self.command.run(client, events).await
    }
}

impl ControlCommands {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        match self {
            ControlCommands::Zone(zone) => zone.run(client, events).await,

            ControlCommands::Image(image) => image.run(client, events).await,

            ControlCommands::Device(device) => device.run(client, events).await,

            ControlCommands::Host(host) => host.run(client, events).await,
        }
    }
}

pub async fn resolve_zone(
    client: &mut ControlServiceClient<Channel>,
    name: &str,
) -> Result<String> {
    let reply = client
        .resolve_zone_id(Request::new(ResolveZoneIdRequest {
            name: name.to_string(),
        }))
        .await?
        .into_inner();

    if !reply.zone_id.is_empty() {
        Ok(reply.zone_id)
    } else {
        Err(anyhow!("unable to resolve zone '{}'", name))
    }
}
