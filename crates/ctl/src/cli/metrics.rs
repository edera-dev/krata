use anyhow::Result;
use clap::{Parser, ValueEnum};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Table};
use krata::{
    events::EventStream,
    v1::control::{
        control_service_client::ControlServiceClient, GuestMetrics, ReadGuestMetricsRequest,
    },
};

use tonic::transport::Channel;

use crate::format::{kv2line, proto2dynamic, proto2kv};

use super::resolve_guest;

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum MetricsFormat {
    Table,
    Json,
    JsonPretty,
    Yaml,
    KeyValue,
}

#[derive(Parser)]
#[command(about = "Read metrics from the guest")]
pub struct MetricsCommand {
    #[arg(short, long, default_value = "table", help = "Output format")]
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
        let metrics = client
            .read_guest_metrics(ReadGuestMetricsRequest { guest_id })
            .await?
            .into_inner()
            .metrics
            .unwrap_or_default();
        match self.format {
            MetricsFormat::Table => {
                self.print_metrics_table(metrics)?;
            }

            MetricsFormat::Json | MetricsFormat::JsonPretty | MetricsFormat::Yaml => {
                let value = serde_json::to_value(proto2dynamic(metrics)?)?;
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
                self.print_key_value(metrics)?;
            }
        }

        Ok(())
    }

    fn print_metrics_table(&self, metrics: GuestMetrics) -> Result<()> {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        table.set_header(vec!["metric", "value"]);
        let kvs = proto2kv(metrics)?;
        for (key, value) in kvs {
            table.add_row(vec![key, value]);
        }
        println!("{}", table);
        Ok(())
    }

    fn print_key_value(&self, metrics: GuestMetrics) -> Result<()> {
        let kvs = proto2kv(metrics)?;
        println!("{}", kv2line(kvs),);
        Ok(())
    }
}
