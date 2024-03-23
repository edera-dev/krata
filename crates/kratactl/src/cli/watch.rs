use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::{common::Guest, control::watch_events_reply::Event};
use prost_reflect::ReflectMessage;
use serde_json::Value;

use crate::{
    cli::pretty::guest_state_text,
    events::EventStream,
    format::{kv2line, proto2dynamic, proto2kv},
};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum WatchFormat {
    Simple,
    Json,
    KeyValue,
}

#[derive(Parser)]
pub struct WatchCommand {
    #[arg(short, long, default_value = "simple")]
    format: WatchFormat,
}

impl WatchCommand {
    pub async fn run(self, events: EventStream) -> Result<()> {
        let mut stream = events.subscribe();
        loop {
            let event = stream.recv().await?;
            match event {
                Event::GuestChanged(changed) => {
                    let guest = changed.guest.clone();
                    self.print_event("guest.changed", changed, guest)?;
                }
            }
        }
    }

    fn print_event(
        &self,
        typ: &str,
        event: impl ReflectMessage,
        guest: Option<Guest>,
    ) -> Result<()> {
        match self.format {
            WatchFormat::Simple => {
                if let Some(guest) = guest {
                    println!(
                        "{} guest={} status=\"{}\"",
                        typ,
                        guest.id,
                        guest_state_text(guest.state.as_ref()).replace('"', "\\\"")
                    );
                }
            }

            WatchFormat::Json => {
                let message = proto2dynamic(event)?;
                let mut value = serde_json::to_value(&message)?;
                if let Value::Object(ref mut map) = value {
                    map.insert("event.type".to_string(), Value::String(typ.to_string()));
                }
                println!("{}", serde_json::to_string(&value)?);
            }

            WatchFormat::KeyValue => {
                let mut map = proto2kv(event)?;
                map.insert("event.type".to_string(), typ.to_string());
                println!("{}", kv2line(map),);
            }
        }
        Ok(())
    }
}
