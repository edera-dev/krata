use prost_types::{ListValue, Value};

include!(concat!(env!("OUT_DIR"), "/krata.internal.idm.rs"));

pub trait AsIdmMetricValue {
    fn as_metric_value(&self) -> Value;
}

impl IdmMetricNode {
    pub fn structural<N: AsRef<str>>(name: N, children: Vec<IdmMetricNode>) -> IdmMetricNode {
        IdmMetricNode {
            name: name.as_ref().to_string(),
            value: None,
            format: IdmMetricFormat::Unknown.into(),
            children,
        }
    }

    pub fn raw_value<N: AsRef<str>, V: AsIdmMetricValue>(name: N, value: V) -> IdmMetricNode {
        IdmMetricNode {
            name: name.as_ref().to_string(),
            value: Some(value.as_metric_value()),
            format: IdmMetricFormat::Unknown.into(),
            children: vec![],
        }
    }

    pub fn value<N: AsRef<str>, V: AsIdmMetricValue>(
        name: N,
        value: V,
        format: IdmMetricFormat,
    ) -> IdmMetricNode {
        IdmMetricNode {
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
