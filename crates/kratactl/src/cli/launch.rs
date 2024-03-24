use std::collections::HashMap;

use anyhow::Result;
use clap::Parser;
use krata::v1::{
    common::{
        guest_image_spec::Image, GuestEnvVar, GuestImageSpec, GuestOciImageSpec, GuestSpec,
        GuestStatus,
    },
    control::{
        control_service_client::ControlServiceClient, watch_events_reply::Event, CreateGuestRequest,
    },
};
use log::error;
use tokio::select;
use tonic::{transport::Channel, Request};

use crate::{console::StdioConsoleStream, events::EventStream};

#[derive(Parser)]
pub struct LauchCommand {
    #[arg(short, long)]
    name: Option<String>,
    #[arg(short, long, default_value_t = 1)]
    cpus: u32,
    #[arg(short, long, default_value_t = 512)]
    mem: u64,
    #[arg[short, long]]
    env: Option<Vec<String>>,
    #[arg(short, long)]
    attach: bool,
    #[arg(short = 'W', long)]
    wait: bool,
    #[arg()]
    oci: String,
    #[arg(allow_hyphen_values = true, trailing_var_arg = true)]
    run: Vec<String>,
}

impl LauchCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let request = CreateGuestRequest {
            spec: Some(GuestSpec {
                name: self.name.unwrap_or_default(),
                image: Some(GuestImageSpec {
                    image: Some(Image::Oci(GuestOciImageSpec { image: self.oci })),
                }),
                vcpus: self.cpus,
                mem: self.mem,
                env: env_map(&self.env.unwrap_or_default())
                    .iter()
                    .map(|(key, value)| GuestEnvVar {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect(),
                run: self.run,
            }),
        };
        let response = client
            .create_guest(Request::new(request))
            .await?
            .into_inner();
        let id = response.guest_id;

        if self.wait || self.attach {
            wait_guest_started(&id, events.clone()).await?;
        }

        let code = if self.attach {
            let input = StdioConsoleStream::stdin_stream(id.clone()).await;
            let output = client.console_data(input).await?.into_inner();
            let stdout_handle =
                tokio::task::spawn(async move { StdioConsoleStream::stdout(output).await });
            let exit_hook_task = StdioConsoleStream::guest_exit_hook(id.clone(), events).await?;
            select! {
                x = stdout_handle => {
                    x??;
                    None
                },
                x = exit_hook_task => x?
            }
        } else {
            println!("{}", id);
            None
        };
        StdioConsoleStream::restore_terminal_mode();
        std::process::exit(code.unwrap_or(0));
    }
}

async fn wait_guest_started(id: &str, events: EventStream) -> Result<()> {
    let mut stream = events.subscribe();
    while let Ok(event) = stream.recv().await {
        match event {
            Event::GuestChanged(changed) => {
                let Some(guest) = changed.guest else {
                    continue;
                };

                if guest.id != id {
                    continue;
                }

                let Some(state) = guest.state else {
                    continue;
                };

                if let Some(ref error) = state.error_info {
                    if state.status() == GuestStatus::Failed {
                        error!("launch failed: {}", error.message);
                        std::process::exit(1);
                    } else {
                        error!("guest error: {}", error.message);
                    }
                }

                if state.status() == GuestStatus::Destroyed {
                    error!("guest destroyed");
                    std::process::exit(1);
                }

                if state.status() == GuestStatus::Started {
                    break;
                }
            }
        }
    }
    Ok(())
}

fn env_map(env: &[String]) -> HashMap<String, String> {
    let mut map = HashMap::<String, String>::new();
    for item in env {
        if let Some((key, value)) = item.split_once('=') {
            map.insert(key.to_string(), value.to_string());
        }
    }
    map
}
