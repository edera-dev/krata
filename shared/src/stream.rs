use crate::control::{Message, StreamStatus, StreamUpdate, StreamUpdated};
use anyhow::{anyhow, Result};
use log::warn;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{
    mpsc::{channel, Receiver, Sender},
    Mutex,
};

pub struct StreamContext {
    pub id: u64,
    pub receiver: Receiver<StreamUpdate>,
    sender: Sender<Message>,
}

impl StreamContext {
    pub async fn send(&self, update: StreamUpdate) -> Result<()> {
        self.sender
            .send(Message::StreamUpdated(StreamUpdated {
                id: self.id,
                update: Some(update),
                status: StreamStatus::Open,
            }))
            .await?;
        Ok(())
    }
}

impl Drop for StreamContext {
    fn drop(&mut self) {
        if self.sender.is_closed() {
            return;
        }
        let result = self.sender.try_send(Message::StreamUpdated(StreamUpdated {
            id: self.id,
            update: None,
            status: StreamStatus::Closed,
        }));

        if let Err(error) = result {
            warn!(
                "failed to send close message for stream {}: {}",
                self.id, error
            );
        }
    }
}

struct StreamStorage {
    rx_sender: Sender<StreamUpdate>,
    rx_receiver: Option<Receiver<StreamUpdate>>,
}

#[derive(Clone)]
pub struct ConnectionStreams {
    next: Arc<Mutex<u64>>,
    streams: Arc<Mutex<HashMap<u64, StreamStorage>>>,
    tx_sender: Sender<Message>,
}

const QUEUE_MAX_LEN: usize = 100;

impl ConnectionStreams {
    pub fn new(tx_sender: Sender<Message>) -> Self {
        Self {
            next: Arc::new(Mutex::new(0)),
            streams: Arc::new(Mutex::new(HashMap::new())),
            tx_sender,
        }
    }

    pub async fn open(&self) -> Result<StreamContext> {
        let id = {
            let mut next = self.next.lock().await;
            let id = *next;
            *next = id + 1;
            id
        };

        let (rx_sender, rx_receiver) = channel(QUEUE_MAX_LEN);
        let store = StreamStorage {
            rx_sender,
            rx_receiver: None,
        };

        self.streams.lock().await.insert(id, store);

        let open = Message::StreamUpdated(StreamUpdated {
            id,
            update: None,
            status: StreamStatus::Open,
        });
        self.tx_sender.send(open).await?;

        Ok(StreamContext {
            id,
            sender: self.tx_sender.clone(),
            receiver: rx_receiver,
        })
    }

    pub async fn incoming(&self, updated: StreamUpdated) -> Result<()> {
        let mut streams = self.streams.lock().await;
        if updated.update.is_none() && updated.status == StreamStatus::Open {
            let (rx_sender, rx_receiver) = channel(QUEUE_MAX_LEN);
            let store = StreamStorage {
                rx_sender,
                rx_receiver: Some(rx_receiver),
            };
            streams.insert(updated.id, store);
        }

        let Some(storage) = streams.get(&updated.id) else {
            return Ok(());
        };

        if let Some(update) = updated.update {
            storage.rx_sender.send(update).await?;
        }

        if updated.status == StreamStatus::Closed {
            streams.remove(&updated.id);
        }

        Ok(())
    }

    pub async fn outgoing(&self, updated: &StreamUpdated) -> Result<()> {
        if updated.status == StreamStatus::Closed {
            let mut streams = self.streams.lock().await;
            streams.remove(&updated.id);
        }
        Ok(())
    }

    pub async fn acquire(&self, id: u64) -> Result<StreamContext> {
        let mut streams = self.streams.lock().await;
        let Some(storage) = streams.get_mut(&id) else {
            return Err(anyhow!("stream {} has not been opened", id));
        };

        let Some(receiver) = storage.rx_receiver.take() else {
            return Err(anyhow!("stream has already been acquired"));
        };

        Ok(StreamContext {
            id,
            receiver,
            sender: self.tx_sender.clone(),
        })
    }
}
