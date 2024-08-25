use crate::event::DaemonEventContext;
use anyhow::Result;
use async_stream::try_stream;
use krata::v1::control::{WatchEventsReply, WatchEventsRequest};
use std::pin::Pin;
use tokio_stream::Stream;
use tonic::Status;

pub struct WatchEventsRpc {
    events: DaemonEventContext,
}

impl WatchEventsRpc {
    pub fn new(events: DaemonEventContext) -> Self {
        Self { events }
    }

    pub async fn process(
        self,
        _request: WatchEventsRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<WatchEventsReply, Status>> + Send + 'static>>>
    {
        let mut events = self.events.subscribe();
        let output = try_stream! {
            while let Ok(event) = events.recv().await {
                yield WatchEventsReply { event: Some(event), };
            }
        };
        Ok(Box::pin(output))
    }
}
