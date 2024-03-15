use anyhow::Result;
use clap::Parser;
use krata::{
    common::{guest_image_spec::Image, GuestImageSpec, GuestOciImageSpec, GuestSpec, GuestStatus},
    control::{
        control_service_client::ControlServiceClient, watch_events_reply::Event, CreateGuestRequest,
    },
};
use log::error;
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
                env: self.env.unwrap_or_default(),
                run: self.run,
            }),
        };
        let response = client
            .create_guest(Request::new(request))
            .await?
            .into_inner();
        let id = response.guest_id;
        if self.attach {
            wait_guest_started(&id, events.clone()).await?;
            let input = StdioConsoleStream::stdin_stream(id.clone()).await;
            let output = client.console_data(input).await?.into_inner();
            let exit_hook_task = StdioConsoleStream::guest_exit_hook(id.clone(), events).await?;
            StdioConsoleStream::stdout(output).await?;
            exit_hook_task.abort();
        } else {
            println!("created guest: {}", id);
        }
        Ok(())
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
                    error!("guest error: {}", error.message);
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
