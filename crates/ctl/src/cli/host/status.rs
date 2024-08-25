use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::v1::control::{control_service_client::ControlServiceClient, GetHostStatusRequest};

use crate::format::{kv2line, proto2dynamic, proto2kv};
use tonic::{transport::Channel, Request};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum HostStatusFormat {
    Simple,
    Json,
    JsonPretty,
    Yaml,
    KeyValue,
}

#[derive(Parser)]
#[command(about = "Get information about the host")]
pub struct HostStatusCommand {
    #[arg(short, long, default_value = "simple", help = "Output format")]
    format: HostStatusFormat,
}

impl HostStatusCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let response = client
            .get_host_status(Request::new(GetHostStatusRequest {}))
            .await?
            .into_inner();
        match self.format {
            HostStatusFormat::Simple => {
                println!("Host UUID: {}", response.host_uuid);
                println!("Host Domain: {}", response.host_domid);
                println!("Krata Version: {}", response.krata_version);
                println!("Host IPv4: {}", response.host_ipv4);
                println!("Host IPv6: {}", response.host_ipv6);
                println!("Host Ethernet Address: {}", response.host_mac);
            }

            HostStatusFormat::Json | HostStatusFormat::JsonPretty | HostStatusFormat::Yaml => {
                let message = proto2dynamic(response)?;
                let value = serde_json::to_value(message)?;
                let encoded = if self.format == HostStatusFormat::JsonPretty {
                    serde_json::to_string_pretty(&value)?
                } else if self.format == HostStatusFormat::Yaml {
                    serde_yaml::to_string(&value)?
                } else {
                    serde_json::to_string(&value)?
                };
                println!("{}", encoded.trim());
            }

            HostStatusFormat::KeyValue => {
                let kvs = proto2kv(response)?;
                println!("{}", kv2line(kvs),);
            }
        }
        Ok(())
    }
}
