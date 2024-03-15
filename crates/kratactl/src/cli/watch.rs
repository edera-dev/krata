use anyhow::Result;
use clap::Parser;
use krata::control::watch_events_reply::Event;

use crate::{cli::pretty::guest_status_text, events::EventStream};

#[derive(Parser)]
pub struct WatchCommand {}

impl WatchCommand {
    pub async fn run(self, events: EventStream) -> Result<()> {
        let mut stream = events.subscribe();
        loop {
            let event = stream.recv().await?;
            match event {
                Event::GuestChanged(changed) => {
                    if let Some(guest) = changed.guest {
                        println!(
                            "event=guest.changed guest={} status={}",
                            guest.id,
                            guest_status_text(guest.state.unwrap_or_default().status())
                        );
                    }
                }
            }
        }
    }
}
