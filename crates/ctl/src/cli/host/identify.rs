use anyhow::Result;
use clap::Parser;
use krata::v1::control::{control_service_client::ControlServiceClient, HostStatusRequest};

use tonic::{transport::Channel, Request};

#[derive(Parser)]
#[command(about = "Get information about the host")]
pub struct HostStatusCommand {}

impl HostStatusCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let response = client
            .host_status(Request::new(HostStatusRequest {}))
            .await?
            .into_inner();
        println!("Host UUID: {}", response.host_uuid);
        println!("Host Domain: {}", response.host_domid);
        println!("Krata Version: {}", response.krata_version);
        println!("Host IPv4: {}", response.host_ipv4);
        println!("Host IPv6: {}", response.host_ipv6);
        println!("Host Ethernet Address: {}", response.host_mac);
        Ok(())
    }
}
