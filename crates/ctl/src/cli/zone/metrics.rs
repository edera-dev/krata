use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::{
    events::EventStream,
    v1::{
        common::ZoneMetricNode,
        control::{control_service_client::ControlServiceClient, ReadZoneMetricsRequest},
    },
};

use tonic::transport::Channel;

use crate::format::{kv2line, metrics_flat, metrics_tree, proto2dynamic};

use crate::cli::resolve_zone;

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum ZoneMetricsFormat {
    Tree,
    Json,
    JsonPretty,
    Yaml,
    KeyValue,
}

#[derive(Parser)]
#[command(about = "Read metrics from the zone")]
pub struct ZoneMetricsCommand {
    #[arg(short, long, default_value = "tree", help = "Output format")]
    format: ZoneMetricsFormat,
    #[arg(help = "Zone to read metrics for, either the name or the uuid")]
    zone: String,
}

impl ZoneMetricsCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let zone_id: String = resolve_zone(&mut client, &self.zone).await?;
        let root = client
            .read_zone_metrics(ReadZoneMetricsRequest { zone_id })
            .await?
            .into_inner()
            .root
            .unwrap_or_default();
        match self.format {
            ZoneMetricsFormat::Tree => {
                self.print_metrics_tree(root)?;
            }

            ZoneMetricsFormat::Json | ZoneMetricsFormat::JsonPretty | ZoneMetricsFormat::Yaml => {
                let value = serde_json::to_value(proto2dynamic(root)?)?;
                let encoded = if self.format == ZoneMetricsFormat::JsonPretty {
                    serde_json::to_string_pretty(&value)?
                } else if self.format == ZoneMetricsFormat::Yaml {
                    serde_yaml::to_string(&value)?
                } else {
                    serde_json::to_string(&value)?
                };
                println!("{}", encoded.trim());
            }

            ZoneMetricsFormat::KeyValue => {
                self.print_key_value(root)?;
            }
        }

        Ok(())
    }

    fn print_metrics_tree(&self, root: ZoneMetricNode) -> Result<()> {
        print!("{}", metrics_tree(root));
        Ok(())
    }

    fn print_key_value(&self, metrics: ZoneMetricNode) -> Result<()> {
        let kvs = metrics_flat(metrics);
        println!("{}", kv2line(kvs));
        Ok(())
    }
}
