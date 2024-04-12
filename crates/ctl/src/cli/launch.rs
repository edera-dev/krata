use std::collections::HashMap;

use anyhow::Result;
use clap::Parser;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use krata::{
    events::EventStream,
    v1::{
        common::{
            guest_image_spec::Image, GuestImageSpec, GuestOciImageSpec, GuestSpec, GuestStatus,
            GuestTaskSpec, GuestTaskSpecEnvVar,
        },
        control::{
            control_service_client::ControlServiceClient, watch_events_reply::Event,
            CreateGuestRequest, OciProgressEventLayerPhase, OciProgressEventPhase,
        },
    },
};
use log::error;
use tokio::select;
use tonic::{transport::Channel, Request};

use crate::console::StdioConsoleStream;

#[derive(Parser)]
#[command(about = "Launch a new guest")]
pub struct LauchCommand {
    #[arg(short, long, help = "Name of the guest")]
    name: Option<String>,
    #[arg(
        short,
        long,
        default_value_t = 1,
        help = "vCPUs available to the guest"
    )]
    cpus: u32,
    #[arg(
        short,
        long,
        default_value_t = 512,
        help = "Memory available to the guest, in megabytes"
    )]
    mem: u64,
    #[arg[short, long, help = "Environment variables set in the guest"]]
    env: Option<Vec<String>>,
    #[arg(
        short,
        long,
        help = "Attach to the guest after guest starts, implies --wait"
    )]
    attach: bool,
    #[arg(
        short = 'W',
        long,
        help = "Wait for the guest to start, implied by --attach"
    )]
    wait: bool,
    #[arg(help = "Container image for guest to use")]
    oci: String,
    #[arg(
        allow_hyphen_values = true,
        trailing_var_arg = true,
        help = "Command to run inside the guest"
    )]
    command: Vec<String>,
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
                task: Some(GuestTaskSpec {
                    environment: env_map(&self.env.unwrap_or_default())
                        .iter()
                        .map(|(key, value)| GuestTaskSpecEnvVar {
                            key: key.clone(),
                            value: value.clone(),
                        })
                        .collect(),
                    command: self.command,
                }),
                annotations: vec![],
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
    let mut multi_progress: Option<(MultiProgress, HashMap<String, ProgressBar>)> = None;
    while let Ok(event) = stream.recv().await {
        match event {
            Event::GuestChanged(changed) => {
                if let Some((multi_progress, _)) = multi_progress.as_mut() {
                    let _ = multi_progress.clear();
                }

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

            Event::OciProgress(oci) => {
                if multi_progress.is_none() {
                    multi_progress = Some((MultiProgress::new(), HashMap::new()));
                }

                let Some((multi_progress, progresses)) = multi_progress.as_mut() else {
                    continue;
                };

                match oci.phase() {
                    OciProgressEventPhase::Resolved
                    | OciProgressEventPhase::ConfigAcquire
                    | OciProgressEventPhase::LayerAcquire => {
                        if progresses.is_empty() && !oci.layers.is_empty() {
                            for layer in &oci.layers {
                                let bar = ProgressBar::new(layer.total);
                                bar.set_style(
                                    ProgressStyle::with_template("{msg} {wide_bar} {pos}/{len}")
                                        .unwrap(),
                                );
                                progresses.insert(layer.id.clone(), bar.clone());
                                multi_progress.add(bar);
                            }
                        }

                        for layer in oci.layers {
                            let Some(progress) = progresses.get_mut(&layer.id) else {
                                continue;
                            };

                            let phase = match layer.phase() {
                                OciProgressEventLayerPhase::Waiting => "waiting",
                                OciProgressEventLayerPhase::Downloading => "downloading",
                                OciProgressEventLayerPhase::Downloaded => "downloaded",
                                OciProgressEventLayerPhase::Extracting => "extracting",
                                OciProgressEventLayerPhase::Extracted => "extracted",
                                _ => "unknown",
                            };

                            progress.set_message(format!("{} {}", layer.id, phase));
                            progress.set_length(layer.total);
                            progress.set_position(layer.value);
                        }
                    }

                    OciProgressEventPhase::Packing => {
                        for (key, progress) in &mut *progresses {
                            if key == "packing" {
                                continue;
                            }
                            progress.finish_and_clear();
                            multi_progress.remove(progress);
                        }
                        progresses.retain(|k, _| k == "packing");
                        if progresses.is_empty() {
                            let progress = ProgressBar::new(100);
                            progress.set_style(
                                ProgressStyle::with_template("{msg} {wide_bar} {pos}/{len}")
                                    .unwrap(),
                            );
                            progresses.insert("packing".to_string(), progress);
                        }
                        let Some(progress) = progresses.get("packing") else {
                            continue;
                        };
                        progress.set_message("packing image");
                        progress.set_length(oci.total);
                        progress.set_position(oci.value);
                    }

                    _ => {}
                }

                for progress in progresses {
                    progress.1.tick();
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
