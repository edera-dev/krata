use std::collections::HashMap;

use anyhow::Result;
use krata::common::{GuestState, GuestStatus};
use prost_reflect::{DynamicMessage, ReflectMessage, Value};

pub fn proto2dynamic(proto: impl ReflectMessage) -> Result<DynamicMessage> {
    Ok(DynamicMessage::decode(
        proto.descriptor(),
        proto.encode_to_vec().as_slice(),
    )?)
}

pub fn proto2kv(proto: impl ReflectMessage) -> Result<HashMap<String, String>> {
    let message = proto2dynamic(proto)?;
    let mut map = HashMap::new();

    fn crawl(prefix: &str, map: &mut HashMap<String, String>, message: &DynamicMessage) {
        for (field, value) in message.fields() {
            let path = if prefix.is_empty() {
                field.name().to_string()
            } else {
                format!("{}.{}", prefix, field.name())
            };
            match value {
                Value::Message(child) => {
                    crawl(&path, map, child);
                }

                Value::EnumNumber(number) => {
                    if let Some(e) = field.kind().as_enum() {
                        if let Some(value) = e.get_value(*number) {
                            map.insert(path, value.name().to_string());
                        }
                    }
                }

                Value::String(value) => {
                    map.insert(path, value.clone());
                }

                _ => {
                    map.insert(path, value.to_string());
                }
            }
        }
    }

    crawl("", &mut map, &message);

    Ok(map)
}

pub fn kv2line(map: HashMap<String, String>) -> String {
    map.iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, v.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn guest_status_text(status: GuestStatus) -> String {
    match status {
        GuestStatus::Starting => "starting",
        GuestStatus::Started => "started",
        GuestStatus::Destroying => "destroying",
        GuestStatus::Destroyed => "destroyed",
        GuestStatus::Exited => "exited",
        GuestStatus::Failed => "failed",
        _ => "unknown",
    }
    .to_string()
}

pub fn guest_state_text(state: Option<&GuestState>) -> String {
    let state = state.cloned().unwrap_or_default();
    let mut text = guest_status_text(state.status());

    if let Some(exit) = state.exit_info {
        text.push_str(&format!(" (exit code: {})", exit.code));
    }

    if let Some(error) = state.error_info {
        text.push_str(&format!(" (error: {})", error.message));
    }
    text
}
