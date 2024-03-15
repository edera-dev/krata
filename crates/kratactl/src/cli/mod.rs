pub mod console;
pub mod destroy;
pub mod launch;
pub mod list;
pub mod pretty;
pub mod watch;

use anyhow::Result;
use clap::{Parser, Subcommand};
use krata::control::WatchEventsRequest;

use crate::{client::ControlClientProvider, events::EventStream};

use self::{
    console::ConsoleCommand, destroy::DestroyCommand, launch::LauchCommand, list::ListCommand,
    watch::WatchCommand,
};

#[derive(Parser)]
#[command(version, about)]
pub struct ControlCommand {
    #[arg(short, long, default_value = "unix:///var/lib/krata/daemon.socket")]
    connection: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    Launch(LauchCommand),
    Destroy(DestroyCommand),
    List(ListCommand),
    Console(ConsoleCommand),
    Watch(WatchCommand),
}

impl ControlCommand {
    pub async fn run(self) -> Result<()> {
        let mut client = ControlClientProvider::dial(self.connection.parse()?).await?;
        let events = EventStream::open(
            client
                .watch_events(WatchEventsRequest {})
                .await?
                .into_inner(),
        )
        .await?;

        match self.command {
            Commands::Launch(launch) => {
                launch.run(client, events).await?;
            }

            Commands::Destroy(destroy) => {
                destroy.run(client, events).await?;
            }

            Commands::Console(console) => {
                console.run(client, events).await?;
            }

            Commands::List(list) => {
                list.run(client, events).await?;
            }

            Commands::Watch(watch) => {
                watch.run(events).await?;
            }
        }
        Ok(())
    }
}
