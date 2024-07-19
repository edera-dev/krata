use krata::{
    idm::internal::{MetricFormat, MetricNode},
    v1::common::{ZoneMetricFormat, ZoneMetricNode},
};

fn idm_metric_format_to_api(format: MetricFormat) -> ZoneMetricFormat {
    match format {
        MetricFormat::Unknown => ZoneMetricFormat::Unknown,
        MetricFormat::Bytes => ZoneMetricFormat::Bytes,
        MetricFormat::Integer => ZoneMetricFormat::Integer,
        MetricFormat::DurationSeconds => ZoneMetricFormat::DurationSeconds,
    }
}

pub fn idm_metric_to_api(node: MetricNode) -> ZoneMetricNode {
    let format = node.format();
    ZoneMetricNode {
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
