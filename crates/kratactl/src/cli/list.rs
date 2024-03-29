use anyhow::{anyhow, Result};
use clap::{Parser, ValueEnum};
use cli_tables::Table;
use krata::{
    events::EventStream,
    v1::{
        common::{guest_image_spec::Image, Guest},
        control::{
            control_service_client::ControlServiceClient, ListGuestsRequest, ResolveGuestRequest,
        },
    },
};

use serde_json::Value;
use tonic::{transport::Channel, Request};

use crate::format::{guest_state_text, kv2line, proto2dynamic, proto2kv};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum ListFormat {
    CliTable,
    Json,
    JsonPretty,
    Jsonl,
    Yaml,
    KeyValue,
}

#[derive(Parser)]
pub struct ListCommand {
    #[arg(short, long, default_value = "cli-table")]
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
        let guests = if let Some(ref guest) = self.guest {
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

        match self.format {
            ListFormat::CliTable => {
                self.print_guest_table(guests)?;
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
        let header = vec!["name", "uuid", "state", "ipv4", "ipv6", "image"];
        table.push_row(&header)?;
        for guest in guests {
            let ipv4 = guest
                .state
                .as_ref()
                .and_then(|x| x.network.as_ref())
                .map(|x| x.guest_ipv4.as_str())
                .unwrap_or("unknown");
            let ipv6 = guest
                .state
                .as_ref()
                .and_then(|x| x.network.as_ref())
                .map(|x| x.guest_ipv6.as_str())
                .unwrap_or("unknown");
            let Some(spec) = guest.spec else {
                continue;
            };
            let image = spec
                .image
                .map(|x| {
                    x.image
                        .map(|y| match y {
                            Image::Oci(oci) => oci.image,
                        })
                        .unwrap_or("unknown".to_string())
                })
                .unwrap_or("unknown".to_string());
            table.push_row_string(&vec![
                spec.name,
                guest.id,
                format!("{}", guest_state_text(guest.state.as_ref())),
                ipv4.to_string(),
                ipv6.to_string(),
                image,
            ])?;
        }
        if table.num_records() == 1 {
            if self.guest.is_none() {
                println!("no guests have been launched");
            }
        } else {
            println!("{}", table.to_string());
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
