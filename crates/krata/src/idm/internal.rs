use anyhow::Result;
use prost::Message;
use prost_types::{ListValue, Value};

use super::serialize::{IdmRequest, IdmSerializable};

include!(concat!(env!("OUT_DIR"), "/krata.idm.internal.rs"));

pub const INTERNAL_IDM_CHANNEL: u64 = 0;

impl IdmSerializable for Event {
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(self.encode_to_vec())
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(<Self as prost::Message>::decode(bytes)?)
    }
}

impl IdmSerializable for Request {
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(self.encode_to_vec())
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(<Self as prost::Message>::decode(bytes)?)
    }
}

impl IdmRequest for Request {
    type Response = Response;
}

impl IdmSerializable for Response {
    fn encode(&self) -> Result<Vec<u8>> {
        Ok(self.encode_to_vec())
    }

    fn decode(bytes: &[u8]) -> Result<Self> {
        Ok(<Self as prost::Message>::decode(bytes)?)
    }
}

pub trait AsIdmMetricValue {
    fn as_metric_value(&self) -> Value;
}

impl MetricNode {
    pub fn structural<N: AsRef<str>>(name: N, children: Vec<MetricNode>) -> MetricNode {
        MetricNode {
            name: name.as_ref().to_string(),
            value: None,
            format: MetricFormat::Unknown.into(),
            children,
        }
    }

    pub fn raw_value<N: AsRef<str>, V: AsIdmMetricValue>(name: N, value: V) -> MetricNode {
        MetricNode {
            name: name.as_ref().to_string(),
            value: Some(value.as_metric_value()),
            format: MetricFormat::Unknown.into(),
            children: vec![],
        }
    }

    pub fn value<N: AsRef<str>, V: AsIdmMetricValue>(
        name: N,
        value: V,
        format: MetricFormat,
    ) -> MetricNode {
        MetricNode {
            name: name.as_ref().to_string(),
            value: Some(value.as_metric_value()),
            format: format.into(),
            children: vec![],
        }
    }
}

impl AsIdmMetricValue for String {
    fn as_metric_value(&self) -> Value {
        Value {
            kind: Some(prost_types::value::Kind::StringValue(self.to_string())),
        }
    }
}

impl AsIdmMetricValue for &str {
    fn as_metric_value(&self) -> Value {
        Value {
            kind: Some(prost_types::value::Kind::StringValue(self.to_string())),
        }
    }
}

impl AsIdmMetricValue for u64 {
    fn as_metric_value(&self) -> Value {
        numeric(*self as f64)
    }
}

impl AsIdmMetricValue for i64 {
    fn as_metric_value(&self) -> Value {
        numeric(*self as f64)
    }
}

impl AsIdmMetricValue for f64 {
    fn as_metric_value(&self) -> Value {
        numeric(*self)
    }
}

impl<T: AsIdmMetricValue> AsIdmMetricValue for Vec<T> {
    fn as_metric_value(&self) -> Value {
        let values = self.iter().map(|x| x.as_metric_value()).collect::<_>();
        Value {
            kind: Some(prost_types::value::Kind::ListValue(ListValue { values })),
        }
    }
}

fn numeric(value: f64) -> Value {
    Value {
        kind: Some(prost_types::value::Kind::NumberValue(value)),
    }
}
