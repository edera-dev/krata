use anyhow::Result;
use clap::Parser;
use krata::{events::EventStream, v1::control::control_service_client::ControlServiceClient};

use tokio::select;
use tonic::transport::Channel;

use crate::console::StdioConsoleStream;

use crate::cli::resolve_zone;

#[derive(Parser)]
#[command(about = "Attach to the zone console")]
pub struct ZoneAttachCommand {
    #[arg(help = "Zone to attach to, either the name or the uuid")]
    zone: String,
}

impl ZoneAttachCommand {
    pub async fn run(
        self,
        mut client: ControlServiceClient<Channel>,
        events: EventStream,
    ) -> Result<()> {
        let zone_id: String = resolve_zone(&mut client, &self.zone).await?;
        let input = StdioConsoleStream::stdin_stream(zone_id.clone()).await;
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
