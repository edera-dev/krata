use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, Color, Table};
use krata::{
    events::EventStream,
    v1::{
        common::{Guest, GuestStatus},
        control::{
            control_service_client::ControlServiceClient, ListGuestsRequest, ResolveGuestRequest,
        },
    },
};

use serde_json::Value;
use tonic::{transport::Channel, Request};

use crate::format::{guest_simple_line, guest_status_text, kv2line, proto2dynamic, proto2kv};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum ListFormat {
    Table,
    Json,
    JsonPretty,
    Jsonl,
    Yaml,
    KeyValue,
    Simple,
}

#[derive(Parser)]
pub struct ListCommand {
    #[arg(short, long, default_value = "table")]
    format: ListFormat,
    #[arg()]
    guest: Option<String>,
}

impl ListCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let mut guests = if let Some(ref guest) = self.guest {
            let reply = client
                .resolve_guest(Request::new(ResolveGuestRequest {
                    name: guest.clone(),
                }))
                .await?
                .into_inner();
            if let Some(guest) = reply.guest {
                vec![guest]
            } else {
                return Err(anyhow!("unable to resolve guest '{}'", guest));
            }
        } else {
            client
                .list_guests(Request::new(ListGuestsRequest {}))
                .await?
                .into_inner()
                .guests
        };

        guests.sort_by(|a, b| {
            a.spec
                .as_ref()
                .map(|x| x.name.as_str())
                .unwrap_or("")
                .cmp(b.spec.as_ref().map(|x| x.name.as_str()).unwrap_or(""))
        });

        match self.format {
            ListFormat::Table => {
                self.print_guest_table(guests)?;
            }

            ListFormat::Simple => {
                for guest in guests {
                    println!("{}", guest_simple_line(&guest));
                }
            }

            ListFormat::Json | ListFormat::JsonPretty | ListFormat::Yaml => {
                let mut values = Vec::new();
                for guest in guests {
                    let message = proto2dynamic(guest)?;
                    values.push(serde_json::to_value(message)?);
                }
                let value = Value::Array(values);
                let encoded = if self.format == ListFormat::JsonPretty {
                    serde_json::to_string_pretty(&value)?
                } else if self.format == ListFormat::Yaml {
                    serde_yaml::to_string(&value)?
                } else {
                    serde_json::to_string(&value)?
                };
                println!("{}", encoded.trim());
            }

            ListFormat::Jsonl => {
                for guest in guests {
                    let message = proto2dynamic(guest)?;
                    println!("{}", serde_json::to_string(&message)?);
                }
            }

            ListFormat::KeyValue => {
                self.print_key_value(guests)?;
            }
        }

        Ok(())
    }

    fn print_guest_table(&self, guests: Vec<Guest>) -> Result<()> {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        table.set_header(vec!["name", "uuid", "status", "ipv4", "ipv6"]);
        for guest in guests {
            let ipv4 = guest
                .state
                .as_ref()
                .and_then(|x| x.network.as_ref())
                .map(|x| x.guest_ipv4.as_str())
                .unwrap_or("n/a");
            let ipv6 = guest
                .state
                .as_ref()
                .and_then(|x| x.network.as_ref())
                .map(|x| x.guest_ipv6.as_str())
                .unwrap_or("n/a");
            let Some(spec) = guest.spec else {
                continue;
            };
            let status = guest.state.as_ref().cloned().unwrap_or_default().status();
            let status_text = guest_status_text(status);

            let status_color = match status {
                GuestStatus::Destroyed | GuestStatus::Failed => Color::Red,
                GuestStatus::Destroying | GuestStatus::Exited | GuestStatus::Starting => {
                    Color::Yellow
                }
                GuestStatus::Started => Color::Green,
                _ => Color::Reset,
            };

            table.add_row(vec![
                Cell::new(spec.name),
                Cell::new(guest.id),
                Cell::new(status_text).fg(status_color),
                Cell::new(ipv4.to_string()),
                Cell::new(ipv6.to_string()),
            ]);
        }
        if table.is_empty() {
            if self.guest.is_none() {
                println!("no guests have been launched");
            }
        } else {
            println!("{}", table);
        }
        Ok(())
    }

    fn print_key_value(&self, guests: Vec<Guest>) -> Result<()> {
        for guest in guests {
            let kvs = proto2kv(guest)?;
            println!("{}", kv2line(kvs),);
        }
        Ok(())
    }
}
