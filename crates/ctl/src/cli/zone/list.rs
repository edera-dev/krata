use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, Color, Table};
use krata::{
    events::EventStream,
    v1::{
        common::Zone,
        control::{
            control_service_client::ControlServiceClient, ListZonesRequest, ResolveZoneIdRequest,
        },
    },
};

use crate::format::{kv2line, proto2dynamic, proto2kv, zone_simple_line, zone_state_text};
use krata::v1::common::ZoneState;
use krata::v1::control::GetZoneRequest;
use serde_json::Value;
use tonic::{transport::Channel, Request};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum ZoneListFormat {
    Table,
    Json,
    JsonPretty,
    Jsonl,
    Yaml,
    KeyValue,
    Simple,
}

#[derive(Parser)]
#[command(about = "List zone information")]
pub struct ZoneListCommand {
    #[arg(short, long, default_value = "table", help = "Output format")]
    format: ZoneListFormat,
    #[arg(help = "Limit to a single zone, either the name or the uuid")]
    zone: Option<String>,
}

impl ZoneListCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let mut zones = if let Some(ref zone) = self.zone {
            let reply = client
                .resolve_zone_id(Request::new(ResolveZoneIdRequest { name: zone.clone() }))
                .await?
                .into_inner();
            if !reply.zone_id.is_empty() {
                let reply = client
                    .get_zone(Request::new(GetZoneRequest {
                        zone_id: reply.zone_id,
                    }))
                    .await?
                    .into_inner();
                if let Some(zone) = reply.zone {
                    vec![zone]
                } else {
                    return Err(anyhow!("unable to resolve zone '{}'", zone));
                }
            } else {
                return Err(anyhow!("unable to resolve zone '{}'", zone));
            }
        } else {
            client
                .list_zones(Request::new(ListZonesRequest {}))
                .await?
                .into_inner()
                .zones
        };

        zones.sort_by(|a, b| {
            a.spec
                .as_ref()
                .map(|x| x.name.as_str())
                .unwrap_or("")
                .cmp(b.spec.as_ref().map(|x| x.name.as_str()).unwrap_or(""))
        });

        match self.format {
            ZoneListFormat::Table => {
                self.print_zone_table(zones)?;
            }

            ZoneListFormat::Simple => {
                for zone in zones {
                    println!("{}", zone_simple_line(&zone));
                }
            }

            ZoneListFormat::Json | ZoneListFormat::JsonPretty | ZoneListFormat::Yaml => {
                let mut values = Vec::new();
                for zone in zones {
                    let message = proto2dynamic(zone)?;
                    values.push(serde_json::to_value(message)?);
                }
                let value = Value::Array(values);
                let encoded = if self.format == ZoneListFormat::JsonPretty {
                    serde_json::to_string_pretty(&value)?
                } else if self.format == ZoneListFormat::Yaml {
                    serde_yaml::to_string(&value)?
                } else {
                    serde_json::to_string(&value)?
                };
                println!("{}", encoded.trim());
            }

            ZoneListFormat::Jsonl => {
                for zone in zones {
                    let message = proto2dynamic(zone)?;
                    println!("{}", serde_json::to_string(&message)?);
                }
            }

            ZoneListFormat::KeyValue => {
                self.print_key_value(zones)?;
            }
        }

        Ok(())
    }

    fn print_zone_table(&self, zones: Vec<Zone>) -> Result<()> {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        table.set_header(vec!["name", "uuid", "state", "ipv4", "ipv6"]);
        for zone in zones {
            let ipv4 = zone
                .status
                .as_ref()
                .and_then(|x| x.network_status.as_ref())
                .map(|x| x.zone_ipv4.as_str())
                .unwrap_or("n/a");
            let ipv6 = zone
                .status
                .as_ref()
                .and_then(|x| x.network_status.as_ref())
                .map(|x| x.zone_ipv6.as_str())
                .unwrap_or("n/a");
            let Some(spec) = zone.spec else {
                continue;
            };
            let state = zone.status.as_ref().cloned().unwrap_or_default().state();
            let status_text = zone_state_text(state);

            let status_color = match state {
                ZoneState::Destroyed | ZoneState::Failed => Color::Red,
                ZoneState::Destroying | ZoneState::Exited | ZoneState::Creating => Color::Yellow,
                ZoneState::Created => Color::Green,
                _ => Color::Reset,
            };

            table.add_row(vec![
                Cell::new(spec.name),
                Cell::new(zone.id),
                Cell::new(status_text).fg(status_color),
                Cell::new(ipv4.to_string()),
                Cell::new(ipv6.to_string()),
            ]);
        }
        if table.is_empty() {
            if self.zone.is_none() {
                println!("no zones have been launched");
            }
        } else {
            println!("{}", table);
        }
        Ok(())
    }

    fn print_key_value(&self, zones: Vec<Zone>) -> Result<()> {
        for zone in zones {
            let kvs = proto2kv(zone)?;
            println!("{}", kv2line(kvs),);
        }
        Ok(())
    }
}
