use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::{
    events::EventStream,
    v1::{common::Zone, control::watch_events_reply::Event},
};
use prost_reflect::ReflectMessage;
use serde_json::Value;

use crate::format::{kv2line, proto2dynamic, proto2kv, zone_simple_line};

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
enum ZoneWatchFormat {
    Simple,
    Json,
    KeyValue,
}

#[derive(Parser)]
#[command(about = "Watch for zone changes")]
pub struct ZoneWatchCommand {
    #[arg(short, long, default_value = "simple", help = "Output format")]
    format: ZoneWatchFormat,
}

impl ZoneWatchCommand {
    pub async fn run(self, events: EventStream) -> Result<()> {
        let mut stream = events.subscribe();
        loop {
            let event = stream.recv().await?;

            let Event::ZoneChanged(changed) = event;
            let zone = changed.zone.clone();
            self.print_event("zone.changed", changed, zone)?;
        }
    }

    fn print_event(&self, typ: &str, event: impl ReflectMessage, zone: Option<Zone>) -> Result<()> {
        match self.format {
            ZoneWatchFormat::Simple => {
                if let Some(zone) = zone {
                    println!("{}", zone_simple_line(&zone));
                }
            }

            ZoneWatchFormat::Json => {
                let message = proto2dynamic(event)?;
                let mut value = serde_json::to_value(&message)?;
                if let Value::Object(ref mut map) = value {
                    map.insert("event.type".to_string(), Value::String(typ.to_string()));
                }
                println!("{}", serde_json::to_string(&value)?);
            }

            ZoneWatchFormat::KeyValue => {
                let mut map = proto2kv(event)?;
                map.insert("event.type".to_string(), typ.to_string());
                println!("{}", kv2line(map),);
            }
        }
        Ok(())
    }
}
