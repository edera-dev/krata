use krata::{
    idm::internal::{MetricFormat, MetricNode},
    v1::common::{GuestMetricFormat, GuestMetricNode},
};

fn idm_metric_format_to_api(format: MetricFormat) -> GuestMetricFormat {
    match format {
        MetricFormat::Unknown => GuestMetricFormat::Unknown,
        MetricFormat::Bytes => GuestMetricFormat::Bytes,
        MetricFormat::Integer => GuestMetricFormat::Integer,
        MetricFormat::DurationSeconds => GuestMetricFormat::DurationSeconds,
    }
}

pub fn idm_metric_to_api(node: MetricNode) -> GuestMetricNode {
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
