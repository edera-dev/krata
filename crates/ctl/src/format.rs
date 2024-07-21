use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use fancy_duration::FancyDuration;
use human_bytes::human_bytes;
use prost_reflect::{DynamicMessage, ReflectMessage};
use prost_types::Value;
use termtree::Tree;

use krata::v1::common::{Zone, ZoneMetricFormat, ZoneMetricNode, ZoneState};

pub fn proto2dynamic(proto: impl ReflectMessage) -> Result<DynamicMessage> {
    Ok(DynamicMessage::decode(
        proto.descriptor(),
        proto.encode_to_vec().as_slice(),
    )?)
}

pub fn value2kv(value: serde_json::Value) -> Result<HashMap<String, String>> {
    let mut map = HashMap::new();
    fn crawl(prefix: String, map: &mut HashMap<String, String>, value: serde_json::Value) {
        fn dot(prefix: &str, next: String) -> String {
            if prefix.is_empty() {
                next.to_string()
            } else {
                format!("{}.{}", prefix, next)
            }
        }

        match value {
            serde_json::Value::Null => {
                map.insert(prefix, "null".to_string());
            }

            serde_json::Value::String(value) => {
                map.insert(prefix, value);
            }

            serde_json::Value::Bool(value) => {
                map.insert(prefix, value.to_string());
            }

            serde_json::Value::Number(value) => {
                map.insert(prefix, value.to_string());
            }

            serde_json::Value::Array(value) => {
                for (i, item) in value.into_iter().enumerate() {
                    let next = dot(&prefix, i.to_string());
                    crawl(next, map, item);
                }
            }

            serde_json::Value::Object(value) => {
                for (key, item) in value {
                    let next = dot(&prefix, key);
                    crawl(next, map, item);
                }
            }
        }
    }
    crawl("".to_string(), &mut map, value);
    Ok(map)
}

pub fn proto2kv(proto: impl ReflectMessage) -> Result<HashMap<String, String>> {
    let message = proto2dynamic(proto)?;
    let value = serde_json::to_value(message)?;
    value2kv(value)
}

pub fn kv2line(map: HashMap<String, String>) -> String {
    map.iter()
        .map(|(k, v)| format!("{}=\"{}\"", k, v.replace('"', "\\\"")))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn zone_state_text(status: ZoneState) -> String {
    match status {
        ZoneState::Creating => "creating",
        ZoneState::Created => "created",
        ZoneState::Destroying => "destroying",
        ZoneState::Destroyed => "destroyed",
        ZoneState::Exited => "exited",
        ZoneState::Failed => "failed",
        _ => "unknown",
    }
    .to_string()
}

pub fn zone_simple_line(zone: &Zone) -> String {
    let state = zone_state_text(
        zone.status
            .as_ref()
            .map(|x| x.state())
            .unwrap_or(ZoneState::Unknown),
    );
    let name = zone.spec.as_ref().map(|x| x.name.as_str()).unwrap_or("");
    let network_status = zone.status.as_ref().and_then(|x| x.network_status.as_ref());
    let ipv4 = network_status.map(|x| x.zone_ipv4.as_str()).unwrap_or("");
    let ipv6 = network_status.map(|x| x.zone_ipv6.as_str()).unwrap_or("");
    format!("{}\t{}\t{}\t{}\t{}", zone.id, state, name, ipv4, ipv6)
}

fn metrics_value_string(value: Value) -> String {
    proto2dynamic(value)
        .map(|x| serde_json::to_string(&x).ok())
        .ok()
        .flatten()
        .unwrap_or_default()
}

fn metrics_value_numeric(value: Value) -> f64 {
    let string = metrics_value_string(value);
    string.parse::<f64>().ok().unwrap_or(f64::NAN)
}

pub fn metrics_value_pretty(value: Value, format: ZoneMetricFormat) -> String {
    match format {
        ZoneMetricFormat::Bytes => human_bytes(metrics_value_numeric(value)),
        ZoneMetricFormat::Integer => (metrics_value_numeric(value) as u64).to_string(),
        ZoneMetricFormat::DurationSeconds => {
            FancyDuration(Duration::from_secs_f64(metrics_value_numeric(value))).to_string()
        }
        _ => metrics_value_string(value),
    }
}

fn metrics_flat_internal(prefix: &str, node: ZoneMetricNode, map: &mut HashMap<String, String>) {
    if let Some(value) = node.value {
        map.insert(prefix.to_string(), metrics_value_string(value));
    }

    for child in node.children {
        let path = if prefix.is_empty() {
            child.name.to_string()
        } else {
            format!("{}.{}", prefix, child.name)
        };
        metrics_flat_internal(&path, child, map);
    }
}

pub fn metrics_flat(root: ZoneMetricNode) -> HashMap<String, String> {
    let mut map = HashMap::new();
    metrics_flat_internal("", root, &mut map);
    map
}

pub fn metrics_tree(node: ZoneMetricNode) -> Tree<String> {
    let mut name = node.name.to_string();
    let format = node.format();
    if let Some(value) = node.value {
        let value_string = metrics_value_pretty(value, format);
        name.push_str(&format!(": {}", value_string));
    }

    let mut tree = Tree::new(name);
    for child in node.children {
        tree.push(metrics_tree(child));
    }
    tree
}
