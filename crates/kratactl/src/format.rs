use std::collections::HashMap;

use anyhow::Result;
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
