use anyhow::Result;
use clap::Parser;
use krata::v1::control::{
    control_service_client::ControlServiceClient, ReadHypervisorConsoleRequest,
};

use tonic::{transport::Channel, Request};

#[derive(Parser)]
#[command(about = "Display hypervisor diagnostic messages")]
pub struct HostHvConsoleCommand {
}

impl HostHvConsoleCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let response = client
            .read_hypervisor_console(Request::new(ReadHypervisorConsoleRequest {}))
            .await?
            .into_inner();

        print!("{}", response.data);
        Ok(())
    }
}
