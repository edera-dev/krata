use anyhow::Result;
use async_stream::stream;
use clap::Parser;
use krata::{
    events::EventStream,
    v1::control::{control_service_client::ControlServiceClient, ZoneConsoleRequest},
};

use tokio::select;
use tokio_stream::{pending, StreamExt};
use tonic::transport::Channel;

use crate::console::StdioConsoleStream;

use super::resolve_zone;

#[derive(Parser)]
#[command(about = "View the logs of a zone")]
pub struct LogsCommand {
    #[arg(short, long, help = "Follow output from the zone")]
    follow: bool,
    #[arg(help = "Zone to show logs for, either the name or the uuid")]
    zone: String,
}

impl LogsCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let zone_id: String = resolve_zone(&mut client, &self.zone).await?;
        let zone_id_stream = zone_id.clone();
        let follow = self.follow;
        let input = stream! {
            yield ZoneConsoleRequest { zone_id: zone_id_stream, data: Vec::new() };
            if follow {
                let mut pending = pending::<ZoneConsoleRequest>();
                while let Some(x) = pending.next().await {
                    yield x;
                }
            }
        };
        let output = client.attach_zone_console(input).await?.into_inner();
        let stdout_handle =
            tokio::task::spawn(async move { StdioConsoleStream::stdout(output).await });
        let exit_hook_task = StdioConsoleStream::zone_exit_hook(zone_id.clone(), events).await?;
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
