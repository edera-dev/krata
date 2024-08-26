use std::collections::HashMap;

use anyhow::Result;

use clap::Parser;
use crossterm::tty::IsTty;
use krata::v1::{
    common::{TerminalSize, ZoneTaskSpec, ZoneTaskSpecEnvVar},
    control::{control_service_client::ControlServiceClient, ExecInsideZoneRequest},
};

use tokio::io::stdin;
use tonic::{transport::Channel, Request};

use crate::console::StdioConsoleStream;

use crate::cli::resolve_zone;

#[derive(Parser)]
#[command(about = "Execute a command inside the zone")]
pub struct ZoneExecCommand {
    #[arg[short, long, help = "Environment variables"]]
    env: Option<Vec<String>>,
    #[arg(short = 'w', long, help = "Working directory")]
    working_directory: Option<String>,
    #[arg(short = 't', long, help = "Allocate tty")]
    tty: bool,
    #[arg(help = "Zone to exec inside, either the name or the uuid")]
    zone: String,
    #[arg(
        allow_hyphen_values = true,
        trailing_var_arg = true,
        help = "Command to run inside the zone"
    )]
    command: Vec<String>,
}

impl ZoneExecCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let zone_id: String = resolve_zone(&mut client, &self.zone).await?;
        let should_map_tty = self.tty && stdin().is_tty();
        let initial = ExecInsideZoneRequest {
            zone_id,
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
                tty: self.tty,
            }),
            stdin: vec![],
            stdin_closed: false,
            terminal_size: if should_map_tty {
                let size = crossterm::terminal::size().ok();
                size.map(|(columns, rows)| TerminalSize {
                    rows: rows as u32,
                    columns: columns as u32,
                })
            } else {
                None
            },
        };

        let stream = StdioConsoleStream::input_stream_exec(initial, should_map_tty).await;

        let response = client
            .exec_inside_zone(Request::new(stream))
            .await?
            .into_inner();

        let code = StdioConsoleStream::exec_output(response, should_map_tty).await?;
        std::process::exit(code);
    }
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
