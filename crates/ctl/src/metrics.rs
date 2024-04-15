use anyhow::Result;
use krata::{
    events::EventStream,
    v1::{
        common::{Guest, GuestMetricNode, GuestStatus},
        control::{
            control_service_client::ControlServiceClient, watch_events_reply::Event,
            ListGuestsRequest, ReadGuestMetricsRequest,
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

use crate::format::metrics_value_pretty;

pub struct MetricState {
    pub guest: Guest,
    pub root: Option<GuestMetricNode>,
}

pub struct MultiMetricState {
    pub guests: Vec<MetricState>,
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
        let mut guests: Vec<Guest> = self
            .client
            .list_guests(ListGuestsRequest {})
            .await?
            .into_inner()
            .guests;
        loop {
            let collect = select! {
                x = events.recv() => match x {
                    Ok(event) => {
                        let Event::GuestChanged(changed) = event;
                            let Some(guest) = changed.guest else {
                                continue;
                            };
                            let Some(ref state) = guest.state else {
                                continue;
                            };
                            guests.retain(|x| x.id != guest.id);
                            if state.status() != GuestStatus::Destroying {
                                guests.push(guest);
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
            for guest in &guests {
                let Some(ref state) = guest.state else {
                    continue;
                };

                if state.status() != GuestStatus::Started {
                    continue;
                }

                let root = timeout(
                    Duration::from_secs(5),
                    self.client.read_guest_metrics(ReadGuestMetricsRequest {
                        guest_id: guest.id.clone(),
                    }),
                )
                .await
                .ok()
                .and_then(|x| x.ok())
                .map(|x| x.into_inner())
                .and_then(|x| x.root);
                metrics.push(MetricState {
                    guest: guest.clone(),
                    root,
                });
            }
            sender.send(MultiMetricState { guests: metrics }).await?;
        }
    }
}

pub fn lookup<'a>(node: &'a GuestMetricNode, path: &str) -> Option<&'a GuestMetricNode> {
    let Some((what, b)) = path.split_once('/') else {
        return node.children.iter().find(|x| x.name == path);
    };
    let next = node.children.iter().find(|x| x.name == what)?;
    return lookup(next, b);
}

pub fn lookup_metric_value(node: &GuestMetricNode, path: &str) -> Option<String> {
    lookup(node, path).and_then(|x| {
        x.value
            .as_ref()
            .map(|v| metrics_value_pretty(v.clone(), x.format()))
    })
}
