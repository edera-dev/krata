use std::{sync::Arc, time::Duration};

use crate::v1::control::{
    control_service_client::ControlServiceClient, watch_events_reply::Event, WatchEventsReply,
    WatchEventsRequest,
};
use anyhow::Result;
use log::{error, trace, warn};
use tokio::{sync::broadcast, task::JoinHandle, time::sleep};
use tokio_stream::StreamExt;
use tonic::{transport::Channel, Streaming};

#[derive(Clone)]
pub struct EventStream {
    sender: Arc<broadcast::Sender<Event>>,
    task: Arc<JoinHandle<()>>,
}

impl EventStream {
    pub async fn open(client: ControlServiceClient<Channel>) -> Result<Self> {
        let (sender, _) = broadcast::channel(1000);
        let emit = sender.clone();
        let task = tokio::task::spawn(async move {
            if let Err(error) = EventStream::process(client, emit).await {
                error!("failed to process event stream: {}", error);
            }
        });
        Ok(Self {
            sender: Arc::new(sender),
            task: Arc::new(task),
        })
    }

    async fn process(
        mut client: ControlServiceClient<Channel>,
        emit: broadcast::Sender<Event>,
    ) -> Result<()> {
        let mut events: Option<Streaming<WatchEventsReply>> = None;
        loop {
            let mut stream = match events {
                Some(stream) => stream,
                None => {
                    let result = client.watch_events(WatchEventsRequest {}).await;
                    if let Err(error) = result {
                        warn!("failed to watch events: {}", error);
                        sleep(Duration::from_secs(1)).await;
                        continue;
                    }
                    result.unwrap().into_inner()
                }
            };

            let Some(result) = stream.next().await else {
                events = None;
                continue;
            };

            let reply = match result {
                Ok(reply) => reply,
                Err(error) => {
                    trace!("event stream processing failed: {}", error);
                    events = None;
                    continue;
                }
            };

            let Some(event) = reply.event else {
                events = Some(stream);
                continue;
            };
            let _ = emit.send(event);
            events = Some(stream);
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.sender.subscribe()
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}
