use anyhow::Result;
use clap::{Parser, Subcommand};
use tonic::transport::Channel;

use krata::events::EventStream;
use krata::v1::control::control_service_client::ControlServiceClient;

use crate::cli::zone::attach::ZoneAttachCommand;
use crate::cli::zone::destroy::ZoneDestroyCommand;
use crate::cli::zone::exec::ZoneExecCommand;
use crate::cli::zone::launch::ZoneLaunchCommand;
use crate::cli::zone::list::ZoneListCommand;
use crate::cli::zone::logs::ZoneLogsCommand;
use crate::cli::zone::metrics::ZoneMetricsCommand;
use crate::cli::zone::resolve::ZoneResolveCommand;
use crate::cli::zone::top::ZoneTopCommand;
use crate::cli::zone::update_resources::ZoneUpdateResourcesCommand;
use crate::cli::zone::watch::ZoneWatchCommand;

pub mod attach;
pub mod destroy;
pub mod exec;
pub mod launch;
pub mod list;
pub mod logs;
pub mod metrics;
pub mod resolve;
pub mod top;
pub mod update_resources;
pub mod watch;

#[derive(Parser)]
#[command(about = "Manage the zones on the isolation engine")]
pub struct ZoneCommand {
    #[command(subcommand)]
    subcommand: ZoneCommands,
}

impl ZoneCommand {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        self.subcommand.run(client, events).await
    }
}

#[allow(clippy::large_enum_variant)]
#[derive(Subcommand)]
pub enum ZoneCommands {
    Attach(ZoneAttachCommand),
    List(ZoneListCommand),
    Launch(ZoneLaunchCommand),
    Destroy(ZoneDestroyCommand),
    Exec(ZoneExecCommand),
    Logs(ZoneLogsCommand),
    Metrics(ZoneMetricsCommand),
    Resolve(ZoneResolveCommand),
    Top(ZoneTopCommand),
    Watch(ZoneWatchCommand),
    UpdateResources(ZoneUpdateResourcesCommand),
}

impl ZoneCommands {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        match self {
            ZoneCommands::Launch(launch) => launch.run(client, events).await,

            ZoneCommands::Destroy(destroy) => destroy.run(client, events).await,

            ZoneCommands::Attach(attach) => attach.run(client, events).await,

            ZoneCommands::Logs(logs) => logs.run(client, events).await,

            ZoneCommands::List(list) => list.run(client, events).await,

            ZoneCommands::Watch(watch) => watch.run(events).await,

            ZoneCommands::Resolve(resolve) => resolve.run(client).await,

            ZoneCommands::Metrics(metrics) => metrics.run(client, events).await,

            ZoneCommands::Top(top) => top.run(client, events).await,

            ZoneCommands::Exec(exec) => exec.run(client).await,

            ZoneCommands::UpdateResources(update_resources) => update_resources.run(client).await,
        }
    }
}
