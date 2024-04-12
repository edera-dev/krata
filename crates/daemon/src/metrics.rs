use krata::{
    idm::protocol::{IdmMetricFormat, IdmMetricNode},
    v1::common::{GuestMetricFormat, GuestMetricNode},
};

fn idm_metric_format_to_api(format: IdmMetricFormat) -> GuestMetricFormat {
    match format {
        IdmMetricFormat::Unknown => GuestMetricFormat::Unknown,
        IdmMetricFormat::Bytes => GuestMetricFormat::Bytes,
        IdmMetricFormat::Integer => GuestMetricFormat::Integer,
        IdmMetricFormat::DurationSeconds => GuestMetricFormat::DurationSeconds,
    }
}

pub fn idm_metric_to_api(node: IdmMetricNode) -> GuestMetricNode {
    let format = node.format();
    GuestMetricNode {
        name: node.name,
        value: node.value,
        format: idm_metric_format_to_api(format).into(),
        children: node
            .children
            .into_iter()
            .map(idm_metric_to_api)
            .collect::<Vec<_>>(),
    }
}
