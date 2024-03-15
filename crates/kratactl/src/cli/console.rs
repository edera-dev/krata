use anyhow::Result;
use clap::Parser;
use krata::control::control_service_client::ControlServiceClient;

use tokio::select;
use tonic::transport::Channel;

use crate::{console::StdioConsoleStream, events::EventStream};

#[derive(Parser)]
pub struct ConsoleCommand {
    #[arg()]
    guest: String,
}

impl ConsoleCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let input = StdioConsoleStream::stdin_stream(self.guest.clone()).await;
        let output = client.console_data(input).await?.into_inner();
        let stdout_handle =
            tokio::task::spawn(async move { StdioConsoleStream::stdout(output).await });
        let exit_hook_task =
            StdioConsoleStream::guest_exit_hook(self.guest.clone(), events).await?;
        let code = select! {
            x = stdout_handle => {
                x??;
                None
            },
            x = exit_hook_task => x?
        };
        std::process::exit(code.unwrap_or(0));
    }
}
