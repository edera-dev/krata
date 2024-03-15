use anyhow::Result;
use clap::Parser;
use krata::control::control_service_client::ControlServiceClient;

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
        let exit_hook_task =
            StdioConsoleStream::guest_exit_hook(self.guest.clone(), events).await?;
        StdioConsoleStream::stdout(output).await?;
        exit_hook_task.abort();
        Ok(())
    }
}
