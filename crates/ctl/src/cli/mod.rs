pub mod attach;
pub mod destroy;
pub mod exec;
pub mod identify_host;
pub mod idm_snoop;
pub mod launch;
pub mod list;
pub mod list_devices;
pub mod logs;
pub mod metrics;
pub mod pull;
pub mod resolve;
pub mod top;
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
    attach::AttachCommand, destroy::DestroyCommand, exec::ExecCommand,
    identify_host::IdentifyHostCommand, idm_snoop::IdmSnoopCommand, launch::LaunchCommand,
    list::ListCommand, list_devices::ListDevicesCommand, logs::LogsCommand,
    metrics::MetricsCommand, pull::PullCommand, resolve::ResolveCommand, top::TopCommand,
    watch::WatchCommand,
};

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
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Launch(LaunchCommand),
    Destroy(DestroyCommand),
    List(ListCommand),
    ListDevices(ListDevicesCommand),
    Attach(AttachCommand),
    Pull(PullCommand),
    Logs(LogsCommand),
    Watch(WatchCommand),
    Resolve(ResolveCommand),
    Metrics(MetricsCommand),
    IdmSnoop(IdmSnoopCommand),
    Top(TopCommand),
    IdentifyHost(IdentifyHostCommand),
    Exec(ExecCommand),
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

            Commands::Logs(logs) => {
                logs.run(client, events).await?;
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

            Commands::Metrics(metrics) => {
                metrics.run(client, events).await?;
            }

            Commands::IdmSnoop(snoop) => {
                snoop.run(client, events).await?;
            }

            Commands::Top(top) => {
                top.run(client, events).await?;
            }

            Commands::Pull(pull) => {
                pull.run(client).await?;
            }

            Commands::IdentifyHost(identify) => {
                identify.run(client).await?;
            }

            Commands::Exec(exec) => {
                exec.run(client).await?;
            }

            Commands::ListDevices(list) => {
                list.run(client, events).await?;
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
