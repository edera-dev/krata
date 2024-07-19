use anyhow::Result;
use clap::Parser;
use krata::v1::control::{control_service_client::ControlServiceClient, IdentifyHostRequest};

use tonic::{transport::Channel, Request};

#[derive(Parser)]
#[command(about = "Identify information about the host")]
pub struct HostIdentifyCommand {}

impl HostIdentifyCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let response = client
            .identify_host(Request::new(IdentifyHostRequest {}))
            .await?
            .into_inner();
        println!("Host UUID: {}", response.host_uuid);
        println!("Host Domain: {}", response.host_domid);
        println!("Krata Version: {}", response.krata_version);
        Ok(())
    }
}
