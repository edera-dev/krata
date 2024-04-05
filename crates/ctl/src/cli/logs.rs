use anyhow::Result;
use async_stream::stream;
use clap::Parser;
use krata::{
    events::EventStream,
    v1::control::{control_service_client::ControlServiceClient, ConsoleDataRequest},
};

use tokio::select;
use tokio_stream::{pending, StreamExt};
use tonic::transport::Channel;

use crate::console::StdioConsoleStream;

use super::resolve_guest;

#[derive(Parser)]
#[command(about = "View the logs of a guest")]
pub struct LogsCommand {
    #[arg(short, long, help = "Follow output from the guest")]
    follow: bool,
    #[arg(help = "Guest to show logs for, either the name or the uuid")]
    guest: String,
}

impl LogsCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let guest_id: String = resolve_guest(&mut client, &self.guest).await?;
        let guest_id_stream = guest_id.clone();
        let follow = self.follow;
        let input = stream! {
            yield ConsoleDataRequest { guest_id: guest_id_stream, data: Vec::new() };
            if follow {
                let mut pending = pending::<ConsoleDataRequest>();
                while let Some(x) = pending.next().await {
                    yield x;
                }
            }
        };
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
