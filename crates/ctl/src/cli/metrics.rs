use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::{
    events::EventStream,
    v1::{
        common::GuestMetricNode,
        control::{control_service_client::ControlServiceClient, ReadGuestMetricsRequest},
    },
};

use tonic::transport::Channel;

use crate::format::{kv2line, metrics_flat, metrics_tree, proto2dynamic};

use super::resolve_guest;

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum MetricsFormat {
    Tree,
    Json,
    JsonPretty,
    Yaml,
    KeyValue,
}

#[derive(Parser)]
#[command(about = "Read metrics from the guest")]
pub struct MetricsCommand {
    #[arg(short, long, default_value = "tree", help = "Output format")]
    format: MetricsFormat,
    #[arg(help = "Guest to read metrics for, either the name or the uuid")]
    guest: String,
}

impl MetricsCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let guest_id: String = resolve_guest(&mut client, &self.guest).await?;
        let root = client
            .read_guest_metrics(ReadGuestMetricsRequest { guest_id })
            .await?
            .into_inner()
            .root
            .unwrap_or_default();
        match self.format {
            MetricsFormat::Tree => {
                self.print_metrics_tree(root)?;
            }

            MetricsFormat::Json | MetricsFormat::JsonPretty | MetricsFormat::Yaml => {
                let value = serde_json::to_value(proto2dynamic(root)?)?;
                let encoded = if self.format == MetricsFormat::JsonPretty {
                    serde_json::to_string_pretty(&value)?
                } else if self.format == MetricsFormat::Yaml {
                    serde_yaml::to_string(&value)?
                } else {
                    serde_json::to_string(&value)?
                };
                println!("{}", encoded.trim());
            }

            MetricsFormat::KeyValue => {
                self.print_key_value(root)?;
            }
        }

        Ok(())
    }

    fn print_metrics_tree(&self, root: GuestMetricNode) -> Result<()> {
        print!("{}", metrics_tree(root));
        Ok(())
    }

    fn print_key_value(&self, metrics: GuestMetricNode) -> Result<()> {
        let kvs = metrics_flat(metrics);
        println!("{}", kv2line(kvs));
        Ok(())
    }
}
