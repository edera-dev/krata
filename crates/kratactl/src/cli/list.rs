use anyhow::Result;
use clap::Parser;
use krata::{
    common::guest_image_spec::Image,
    control::{control_service_client::ControlServiceClient, ListGuestsRequest},
};

use tonic::{transport::Channel, Request};

use crate::events::EventStream;

use super::pretty::guest_state_text;

#[derive(Parser)]
pub struct ListCommand {}

impl ListCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        _events: EventStream,
    ) -> Result<()> {
        let response = client
            .list_guests(Request::new(ListGuestsRequest {}))
            .await?
            .into_inner();
        let mut table = cli_tables::Table::new();
        let header = vec!["name", "uuid", "state", "ipv4", "ipv6", "image"];
        table.push_row(&header)?;
        for guest in response.guests {
            let ipv4 = guest
                .network
                .as_ref()
                .map(|x| x.ipv4.as_str())
                .unwrap_or("unknown");
            let ipv6 = guest
                .network
                .as_ref()
                .map(|x| x.ipv6.as_str())
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
                format!("{}", guest_state_text(guest.state.unwrap_or_default())),
                ipv4.to_string(),
                ipv6.to_string(),
                image,
            ])?;
        }
        if table.num_records() == 1 {
            println!("no guests have been launched");
        } else {
            println!("{}", table.to_string());
        }
        Ok(())
    }
}
