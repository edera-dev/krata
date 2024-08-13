use std::collections::HashMap;

use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::{
    events::EventStream,
    v1::{
        common::{
            zone_image_spec::Image, OciImageFormat, ZoneImageSpec, ZoneOciImageSpec, ZoneSpec,
            ZoneSpecDevice, ZoneStatus, ZoneTaskSpec, ZoneTaskSpecEnvVar,
        },
        control::{
            control_service_client::ControlServiceClient, watch_events_reply::Event,
            CreateZoneRequest, PullImageRequest,
        },
    },
};
use log::error;
use tokio::select;
use tonic::{transport::Channel, Request};

use crate::{console::StdioConsoleStream, pull::pull_interactive_progress};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum LaunchImageFormat {
    Squashfs,
    Erofs,
}

#[derive(Parser)]
#[command(about = "Launch a new zone")]
pub struct ZoneLaunchCommand {
    #[arg(long, default_value = "squashfs", help = "Image format")]
    image_format: LaunchImageFormat,
    #[arg(long, help = "Overwrite image cache on pull")]
    pull_overwrite_cache: bool,
    #[arg(long, help = "Update image on pull")]
    pull_update: bool,
    #[arg(short, long, help = "Name of the zone")]
    name: Option<String>,
    #[arg(short, long, default_value_t = 1, help = "vCPUs available to the zone")]
    cpus: u32,
    #[arg(
        short,
        long,
        default_value_t = 512,
        help = "Memory available to the zone, in megabytes"
    )]
    mem: u64,
    #[arg[short = 'D', long = "device", help = "Devices to request for the zone"]]
    device: Vec<String>,
    #[arg[short, long, help = "Environment variables set in the zone"]]
    env: Option<Vec<String>>,
    #[arg(
        short,
        long,
        help = "Attach to the zone after zone starts, implies --wait"
    )]
    attach: bool,
    #[arg(
        short = 'W',
        long,
        help = "Wait for the zone to start, implied by --attach"
    )]
    wait: bool,
    #[arg(short = 'k', long, help = "OCI kernel image for zone to use")]
    kernel: Option<String>,
    #[arg(short = 'I', long, help = "OCI initrd image for zone to use")]
    initrd: Option<String>,
    #[arg(short = 'w', long, help = "Working directory")]
    working_directory: Option<String>,
    #[arg(help = "Container image for zone to use")]
    oci: String,
    #[arg(
        allow_hyphen_values = true,
        trailing_var_arg = true,
        help = "Command to run inside the zone"
    )]
    command: Vec<String>,
}

impl ZoneLaunchCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let image = self
            .pull_image(
                &mut client,
                &self.oci,
                match self.image_format {
                    LaunchImageFormat::Squashfs => OciImageFormat::Squashfs,
                    LaunchImageFormat::Erofs => OciImageFormat::Erofs,
                },
            )
            .await?;

        let kernel = if let Some(ref kernel) = self.kernel {
            let kernel_image = self
                .pull_image(&mut client, kernel, OciImageFormat::Tar)
                .await?;
            Some(kernel_image)
        } else {
            None
        };

        let initrd = if let Some(ref initrd) = self.initrd {
            let kernel_image = self
                .pull_image(&mut client, initrd, OciImageFormat::Tar)
                .await?;
            Some(kernel_image)
        } else {
            None
        };

        let request = CreateZoneRequest {
            spec: Some(ZoneSpec {
                name: self.name.unwrap_or_default(),
                image: Some(image),
                kernel,
                initrd,
                vcpus: self.cpus,
                mem: self.mem,
                task: Some(ZoneTaskSpec {
                    environment: env_map(&self.env.unwrap_or_default())
                        .iter()
                        .map(|(key, value)| ZoneTaskSpecEnvVar {
                            key: key.clone(),
                            value: value.clone(),
                        })
                        .collect(),
                    command: self.command,
                    working_directory: self.working_directory.unwrap_or_default(),
                }),
                annotations: vec![],
                devices: self
                    .device
                    .iter()
                    .map(|name| ZoneSpecDevice { name: name.clone() })
                    .collect(),
            }),
        };
        let response = client
            .create_zone(Request::new(request))
            .await?
            .into_inner();
        let id = response.zone_id;

        if self.wait || self.attach {
            wait_zone_started(&id, events.clone()).await?;
        }

        let code = if self.attach {
            let input = StdioConsoleStream::stdin_stream(id.clone()).await;
            let output = client.attach_zone_console(input).await?.into_inner();
            let stdout_handle =
                tokio::task::spawn(async move { StdioConsoleStream::stdout(output, true).await });
            let exit_hook_task = StdioConsoleStream::zone_exit_hook(id.clone(), events).await?;
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

    async fn pull_image(
        &self,
        client: &mut ControlServiceClient<Channel>,
        image: &str,
        format: OciImageFormat,
    ) -> Result<ZoneImageSpec> {
        let response = client
            .pull_image(PullImageRequest {
                image: image.to_string(),
                format: format.into(),
                overwrite_cache: self.pull_overwrite_cache,
                update: self.pull_update,
            })
            .await?;
        let reply = pull_interactive_progress(response.into_inner()).await?;
        Ok(ZoneImageSpec {
            image: Some(Image::Oci(ZoneOciImageSpec {
                digest: reply.digest,
                format: reply.format,
            })),
        })
    }
}

async fn wait_zone_started(id: &str, events: EventStream) -> Result<()> {
    let mut stream = events.subscribe();
    while let Ok(event) = stream.recv().await {
        match event {
            Event::ZoneChanged(changed) => {
                let Some(zone) = changed.zone else {
                    continue;
                };

                if zone.id != id {
                    continue;
                }

                let Some(state) = zone.state else {
                    continue;
                };

                if let Some(ref error) = state.error_info {
                    if state.status() == ZoneStatus::Failed {
                        error!("launch failed: {}", error.message);
                        std::process::exit(1);
                    } else {
                        error!("zone error: {}", error.message);
                    }
                }

                if state.status() == ZoneStatus::Destroyed {
                    error!("zone destroyed");
                    std::process::exit(1);
                }

                if state.status() == ZoneStatus::Started {
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
