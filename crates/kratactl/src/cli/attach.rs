use anyhow::Result;
use clap::Parser;
use krata::v1::control::control_service_client::ControlServiceClient;

use tokio::select;
use tonic::transport::Channel;

use crate::{console::StdioConsoleStream, events::EventStream};

use super::resolve_guest;

#[derive(Parser)]
pub struct AttachCommand {
    #[arg()]
    guest: String,
}

impl AttachCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let guest_id: String = resolve_guest(&mut client, &self.guest).await?;
        let input = StdioConsoleStream::stdin_stream(guest_id.clone()).await;
        let output = client.console_data(input).await?.into_inner();
        let stdout_handle =
            tokio::task::spawn(async move { StdioConsoleStream::stdout(output).await });
        let exit_hook_task = StdioConsoleStream::guest_exit_hook(guest_id.clone(), events).await?;
        let code = select! {
            x = stdout_handle => {
                x??;
                None
            },
            x = exit_hook_task => x?
        };
        StdioConsoleStream::restore_terminal_mode();
        std::process::exit(code.unwrap_or(0));
    }
}
