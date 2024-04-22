use std::collections::HashMap;

use anyhow::Result;

use clap::Parser;
use krata::v1::{
    common::{GuestTaskSpec, GuestTaskSpecEnvVar},
    control::{control_service_client::ControlServiceClient, ExecGuestRequest},
};

use tonic::{transport::Channel, Request};

use crate::console::StdioConsoleStream;

use super::resolve_guest;

#[derive(Parser)]
#[command(about = "Execute a command inside the guest")]
pub struct ExecCommand {
    #[arg[short, long, help = "Environment variables"]]
    env: Option<Vec<String>>,
    #[arg(short = 'w', long, help = "Working directory")]
    working_directory: Option<String>,
    #[arg(short = 't', long, help = "Allocate tty")]
    tty: bool,
    #[arg(help = "Guest to exec inside, either the name or the uuid")]
    guest: String,
    #[arg(
        allow_hyphen_values = true,
        trailing_var_arg = true,
        help = "Command to run inside the guest"
    )]
    command: Vec<String>,
}

impl ExecCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let guest_id: String = resolve_guest(&mut client, &self.guest).await?;
        let initial = ExecGuestRequest {
            guest_id,
            task: Some(GuestTaskSpec {
                environment: env_map(&self.env.unwrap_or_default())
                    .iter()
                    .map(|(key, value)| GuestTaskSpecEnvVar {
                        key: key.clone(),
                        value: value.clone(),
                    })
                    .collect(),
                command: self.command,
                working_directory: self.working_directory.unwrap_or_default(),
            }),
            tty: self.tty,
            stdin: vec![],
            stdin_closed: false,
        };

        let stream = StdioConsoleStream::stdin_stream_exec(initial).await;
        let response = client.exec_guest(Request::new(stream)).await?.into_inner();
        let result = StdioConsoleStream::exec_output(self.tty, response).await;
        StdioConsoleStream::restore_terminal_mode();
        let code = result?;
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
