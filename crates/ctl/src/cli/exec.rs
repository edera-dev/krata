use std::collections::HashMap;

use anyhow::Result;

use clap::Parser;
use krata::v1::{
    common::{ZoneTaskSpec, ZoneTaskSpecEnvVar},
    control::{control_service_client::ControlServiceClient, ExecZoneRequest},
};

use tonic::{transport::Channel, Request};

use crate::console::StdioConsoleStream;

use super::resolve_zone;

#[derive(Parser)]
#[command(about = "Execute a command inside the zone")]
pub struct ExecCommand {
    #[arg[short, long, help = "Environment variables"]]
    env: Option<Vec<String>>,
    #[arg(short = 'w', long, help = "Working directory")]
    working_directory: Option<String>,
    #[arg(help = "Zone to exec inside, either the name or the uuid")]
    zone: String,
    #[arg(
        allow_hyphen_values = true,
        trailing_var_arg = true,
        help = "Command to run inside the zone"
    )]
    command: Vec<String>,
}

impl ExecCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let zone_id: String = resolve_zone(&mut client, &self.zone).await?;
        let initial = ExecZoneRequest {
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
            }),
            data: vec![],
        };

        let stream = StdioConsoleStream::stdin_stream_exec(initial).await;

        let response = client.exec_zone(Request::new(stream)).await?.into_inner();

        let code = StdioConsoleStream::exec_output(response).await?;
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
