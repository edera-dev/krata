use anyhow::Result;
use clap::{Parser, ValueEnum};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, Color, Table};
use krata::{
    events::EventStream,
    v1::control::{control_service_client::ControlServiceClient, DeviceInfo, ListDevicesRequest},
};

use serde_json::Value;
use tonic::transport::Channel;

use crate::format::{kv2line, proto2dynamic, proto2kv};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum ListDevicesFormat {
    Table,
    Json,
    JsonPretty,
    Jsonl,
    Yaml,
    KeyValue,
    Simple,
}

#[derive(Parser)]
#[command(about = "List the devices on the hypervisor")]
pub struct ListDevicesCommand {
    #[arg(short, long, default_value = "table", help = "Output format")]
    format: ListDevicesFormat,
}

impl ListDevicesCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let reply = client
            .list_devices(ListDevicesRequest {})
            .await?
            .into_inner();
        let mut devices = reply.devices;

        devices.sort_by(|a, b| a.name.cmp(&b.name));

        match self.format {
            ListDevicesFormat::Table => {
                self.print_devices_table(devices)?;
            }

            ListDevicesFormat::Simple => {
                for device in devices {
                    println!("{}\t{}\t{}", device.name, device.claimed, device.owner);
                }
            }

            ListDevicesFormat::Json | ListDevicesFormat::JsonPretty | ListDevicesFormat::Yaml => {
                let mut values = Vec::new();
                for device in devices {
                    let message = proto2dynamic(device)?;
                    values.push(serde_json::to_value(message)?);
                }
                let value = Value::Array(values);
                let encoded = if self.format == ListDevicesFormat::JsonPretty {
                    serde_json::to_string_pretty(&value)?
                } else if self.format == ListDevicesFormat::Yaml {
                    serde_yaml::to_string(&value)?
                } else {
                    serde_json::to_string(&value)?
                };
                println!("{}", encoded.trim());
            }

            ListDevicesFormat::Jsonl => {
                for device in devices {
                    let message = proto2dynamic(device)?;
                    println!("{}", serde_json::to_string(&message)?);
                }
            }

            ListDevicesFormat::KeyValue => {
                self.print_key_value(devices)?;
            }
        }

        Ok(())
    }

    fn print_devices_table(&self, devices: Vec<DeviceInfo>) -> Result<()> {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        table.set_header(vec!["name", "status", "owner"]);
        for device in devices {
            let status_text = if device.claimed {
                "claimed"
            } else {
                "available"
            };

            let status_color = if device.claimed {
                Color::Blue
            } else {
                Color::Green
            };

            table.add_row(vec![
                Cell::new(device.name),
                Cell::new(status_text).fg(status_color),
                Cell::new(device.owner),
            ]);
        }
        if table.is_empty() {
            println!("no devices configured");
        } else {
            println!("{}", table);
        }
        Ok(())
    }

    fn print_key_value(&self, devices: Vec<DeviceInfo>) -> Result<()> {
        for device in devices {
            let kvs = proto2kv(device)?;
            println!("{}", kv2line(kvs));
        }
        Ok(())
    }
}
