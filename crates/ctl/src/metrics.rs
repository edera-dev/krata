use crate::format::metrics_value_pretty;
use anyhow::Result;
use krata::v1::common::ZoneState;
use krata::{
    events::EventStream,
    v1::{
        common::{Zone, ZoneMetricNode},
        control::{
            control_service_client::ControlServiceClient, watch_events_reply::Event,
            ListZonesRequest, ReadZoneMetricsRequest,
        },
    },
};
use log::error;
use std::time::Duration;
use tokio::{
    select,
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
    time::{sleep, timeout},
};
use tonic::transport::Channel;

pub struct MetricState {
    pub zone: Zone,
    pub root: Option<ZoneMetricNode>,
}

pub struct MultiMetricState {
    pub zones: Vec<MetricState>,
}

pub struct MultiMetricCollector {
    client: ControlServiceClient<Channel>,
    events: EventStream,
    period: Duration,
}

pub struct MultiMetricCollectorHandle {
    pub receiver: Receiver<MultiMetricState>,
    task: JoinHandle<()>,
}

impl Drop for MultiMetricCollectorHandle {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl MultiMetricCollector {
    pub fn new(
        client: ControlServiceClient<Channel>,
        events: EventStream,
        period: Duration,
    ) -> Result<MultiMetricCollector> {
        Ok(MultiMetricCollector {
            client,
            events,
            period,
        })
    }

    pub async fn launch(mut self) -> Result<MultiMetricCollectorHandle> {
        let (sender, receiver) = channel::<MultiMetricState>(100);
        let task = tokio::task::spawn(async move {
            if let Err(error) = self.process(sender).await {
                error!("failed to process multi metric collector: {}", error);
            }
        });
        Ok(MultiMetricCollectorHandle { receiver, task })
    }

    pub async fn process(&mut self, sender: Sender<MultiMetricState>) -> Result<()> {
        let mut events = self.events.subscribe();
        let mut zones: Vec<Zone> = self
            .client
            .list_zones(ListZonesRequest {})
            .await?
            .into_inner()
            .zones;
        loop {
            let collect = select! {
                x = events.recv() => match x {
                    Ok(event) => {
                        let Event::ZoneChanged(changed) = event;
                            let Some(zone) = changed.zone else {
                                continue;
                            };
                            let Some(ref status) = zone.status else {
                                continue;
                            };
                            zones.retain(|x| x.id != zone.id);
                            if status.state() != ZoneState::Destroying {
                                zones.push(zone);
                            }
                        false
                    },

                    Err(error) => {
                        return Err(error.into());
                    }
                },

                _ = sleep(self.period) => {
                    true
                }
            };

            if !collect {
                continue;
            }

            let mut metrics = Vec::new();
            for zone in &zones {
                let Some(ref status) = zone.status else {
                    continue;
                };

                if status.state() != ZoneState::Created {
                    continue;
                }

                let root = timeout(
                    Duration::from_secs(5),
                    self.client.read_zone_metrics(ReadZoneMetricsRequest {
                        zone_id: zone.id.clone(),
                    }),
                )
                .await
                .ok()
                .and_then(|x| x.ok())
                .map(|x| x.into_inner())
                .and_then(|x| x.root);
                metrics.push(MetricState {
                    zone: zone.clone(),
                    root,
                });
            }
            sender.send(MultiMetricState { zones: metrics }).await?;
        }
    }
}

pub fn lookup<'a>(node: &'a ZoneMetricNode, path: &str) -> Option<&'a ZoneMetricNode> {
    let Some((what, b)) = path.split_once('/') else {
        return node.children.iter().find(|x| x.name == path);
    };
    let next = node.children.iter().find(|x| x.name == what)?;
    return lookup(next, b);
}

pub fn lookup_metric_value(node: &ZoneMetricNode, path: &str) -> Option<String> {
    lookup(node, path).and_then(|x| {
        x.value
            .as_ref()
            .map(|v| metrics_value_pretty(v.clone(), x.format()))
    })
}
