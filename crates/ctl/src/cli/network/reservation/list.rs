use anyhow::Result;
use clap::{Parser, ValueEnum};
use comfy_table::{presets::UTF8_FULL_CONDENSED, Cell, Table};
use krata::{
    events::EventStream,
    v1::{
        common::NetworkReservation,
        control::{control_service_client::ControlServiceClient, ListNetworkReservationsRequest},
    },
};

use serde_json::Value;
use tonic::transport::Channel;

use crate::format::{kv2line, proto2dynamic, proto2kv};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum NetworkReservationListFormat {
    Table,
    Json,
    JsonPretty,
    Jsonl,
    Yaml,
    KeyValue,
    Simple,
}

#[derive(Parser)]
#[command(about = "List network reservation information")]
pub struct NetworkReservationListCommand {
    #[arg(short, long, default_value = "table", help = "Output format")]
    format: NetworkReservationListFormat,
}

impl NetworkReservationListCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let reply = client
            .list_network_reservations(ListNetworkReservationsRequest {})
            .await?
            .into_inner();
        let mut reservations = reply.reservations;

        reservations.sort_by(|a, b| a.uuid.cmp(&b.uuid));

        match self.format {
            NetworkReservationListFormat::Table => {
                self.print_reservations_table(reservations)?;
            }

            NetworkReservationListFormat::Simple => {
                for reservation in reservations {
                    println!(
                        "{}\t{}\t{}\t{}",
                        reservation.uuid, reservation.ipv4, reservation.ipv6, reservation.mac
                    );
                }
            }

            NetworkReservationListFormat::Json
            | NetworkReservationListFormat::JsonPretty
            | NetworkReservationListFormat::Yaml => {
                let mut values = Vec::new();
                for device in reservations {
                    let message = proto2dynamic(device)?;
                    values.push(serde_json::to_value(message)?);
                }
                let value = Value::Array(values);
                let encoded = if self.format == NetworkReservationListFormat::JsonPretty {
                    serde_json::to_string_pretty(&value)?
                } else if self.format == NetworkReservationListFormat::Yaml {
                    serde_yaml::to_string(&value)?
                } else {
                    serde_json::to_string(&value)?
                };
                println!("{}", encoded.trim());
            }

            NetworkReservationListFormat::Jsonl => {
                for device in reservations {
                    let message = proto2dynamic(device)?;
                    println!("{}", serde_json::to_string(&message)?);
                }
            }

            NetworkReservationListFormat::KeyValue => {
                self.print_key_value(reservations)?;
            }
        }

        Ok(())
    }

    fn print_reservations_table(&self, reservations: Vec<NetworkReservation>) -> Result<()> {
        let mut table = Table::new();
        table.load_preset(UTF8_FULL_CONDENSED);
        table.set_content_arrangement(comfy_table::ContentArrangement::Dynamic);
        table.set_header(vec!["uuid", "ipv4", "ipv6", "mac"]);
        for reservation in reservations {
            table.add_row(vec![
                Cell::new(reservation.uuid),
                Cell::new(reservation.ipv4),
                Cell::new(reservation.ipv6),
                Cell::new(reservation.mac),
            ]);
        }
        if table.is_empty() {
            println!("no network reservations found");
        } else {
            println!("{}", table);
        }
        Ok(())
    }

    fn print_key_value(&self, reservations: Vec<NetworkReservation>) -> Result<()> {
        for reservation in reservations {
            let kvs = proto2kv(reservation)?;
            println!("{}", kv2line(kvs));
        }
        Ok(())
    }
}
