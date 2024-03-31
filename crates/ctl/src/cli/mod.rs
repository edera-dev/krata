pub mod attach;
pub mod destroy;
pub mod launch;
pub mod list;
pub mod resolve;
pub mod watch;

use anyhow::{anyhow, Result};
use clap::{Parser, Subcommand};
use krata::{
    client::ControlClientProvider,
    events::EventStream,
    v1::control::{control_service_client::ControlServiceClient, ResolveGuestRequest},
};
use tonic::{transport::Channel, Request};

use self::{
    attach::AttachCommand, destroy::DestroyCommand, launch::LauchCommand, list::ListCommand,
    resolve::ResolveCommand, watch::WatchCommand,
};

#[derive(Parser)]
#[command(version, about)]
pub struct ControlCommand {
    #[arg(short, long, default_value = "unix:///var/lib/krata/daemon.socket")]
    connection: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Launch(LauchCommand),
    Destroy(DestroyCommand),
    List(ListCommand),
    Attach(AttachCommand),
    Watch(WatchCommand),
    Resolve(ResolveCommand),
}

impl ControlCommand {
    pub async fn run(self) -> Result<()> {
        let client = ControlClientProvider::dial(self.connection.parse()?).await?;
        let events = EventStream::open(client.clone()).await?;

        match self.command {
            Commands::Launch(launch) => {
                launch.run(client, events).await?;
            }

            Commands::Destroy(destroy) => {
                destroy.run(client, events).await?;
            }

            Commands::Attach(attach) => {
                attach.run(client, events).await?;
            }

            Commands::List(list) => {
                list.run(client, events).await?;
            }

            Commands::Watch(watch) => {
                watch.run(events).await?;
            }

            Commands::Resolve(resolve) => {
                resolve.run(client).await?;
            }
        }
        Ok(())
    }
}

pub async fn resolve_guest(
    client: &mut ControlServiceClient<Channel>,
    name: &str,
) -> Result<String> {
    let reply = client
        .resolve_guest(Request::new(ResolveGuestRequest {
            name: name.to_string(),
        }))
        .await?
        .into_inner();

    if let Some(guest) = reply.guest {
        Ok(guest.id)
    } else {
        Err(anyhow!("unable to resolve guest '{}'", name))
    }
}
