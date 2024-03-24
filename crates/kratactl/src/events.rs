use std::sync::Arc;

use anyhow::Result;
use krata::v1::control::{watch_events_reply::Event, WatchEventsReply};
use log::trace;
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_stream::StreamExt;
use tonic::Streaming;

#[derive(Clone)]
pub struct EventStream {
    sender: Arc<broadcast::Sender<Event>>,
    task: Arc<JoinHandle<()>>,
}

impl EventStream {
    pub async fn open(mut events: Streaming<WatchEventsReply>) -> Result<Self> {
        let (sender, _) = broadcast::channel(1000);
        let emit = sender.clone();
        let task = tokio::task::spawn(async move {
            loop {
                let Some(result) = events.next().await else {
                    break;
                };

                let reply = match result {
                    Ok(reply) => reply,
                    Err(error) => {
                        trace!("event stream processing failed: {}", error);
                        break;
                    }
                };

                let Some(event) = reply.event else {
                    continue;
                };
                let _ = emit.send(event);
            }
        });
        Ok(Self {
            sender: Arc::new(sender),
            task: Arc::new(task),
        })
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
