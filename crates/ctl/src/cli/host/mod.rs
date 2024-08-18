use anyhow::Result;
use clap::{Parser, Subcommand};
use tonic::transport::Channel;

use krata::events::EventStream;
use krata::v1::control::control_service_client::ControlServiceClient;

use crate::cli::host::cpu_topology::HostCpuTopologyCommand;
use crate::cli::host::identify::HostStatusCommand;
use crate::cli::host::idm_snoop::HostIdmSnoopCommand;
use crate::cli::host::dmesg::HostHypervisorMessagesCommand;

pub mod cpu_topology;
pub mod identify;
pub mod idm_snoop;
pub mod dmesg;

#[derive(Parser)]
#[command(about = "Manage the host of the isolation engine")]
pub struct HostCommand {
    #[command(subcommand)]
    subcommand: HostCommands,
}

impl HostCommand {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        self.subcommand.run(client, events).await
    }
}

#[derive(Subcommand)]
pub enum HostCommands {
    CpuTopology(HostCpuTopologyCommand),
    Status(HostStatusCommand),
    IdmSnoop(HostIdmSnoopCommand),
    HypervisorMessages(HostHypervisorMessagesCommand),
}

impl HostCommands {
    pub async fn run(
        self,
        client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        match self {
            HostCommands::CpuTopology(cpu_topology) => cpu_topology.run(client).await,

            HostCommands::Status(status) => status.run(client).await,

            HostCommands::IdmSnoop(snoop) => snoop.run(client, events).await,

            HostCommands::HypervisorMessages(hvdmesg) => hvdmesg.run(client).await,
        }
    }
}
