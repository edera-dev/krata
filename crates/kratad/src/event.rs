use std::{collections::HashMap, time::Duration};

use anyhow::Result;
use krata::{
    common::{GuestExitInfo, GuestState, GuestStatus},
    control::watch_events_reply::Event,
};
use log::error;
use tokio::{
    sync::{broadcast, mpsc::Sender},
    task::JoinHandle,
    time,
};
use uuid::Uuid;

use kratart::{GuestInfo, Runtime};

use crate::db::GuestStore;

pub type DaemonEvent = krata::control::watch_events_reply::Event;

const EVENT_CHANNEL_QUEUE_LEN: usize = 1000;

#[derive(Clone)]
pub struct DaemonEventContext {
    sender: broadcast::Sender<DaemonEvent>,
}

impl DaemonEventContext {
    pub fn subscribe(&self) -> broadcast::Receiver<DaemonEvent> {
        self.sender.subscribe()
    }

    pub fn send(&self, event: DaemonEvent) -> Result<()> {
        let _ = self.sender.send(event);
        Ok(())
    }
}

pub struct DaemonEventGenerator {
    runtime: Runtime,
    guests: GuestStore,
    guest_reconciler_notify: Sender<Uuid>,
    last: HashMap<Uuid, GuestInfo>,
    _sender: broadcast::Sender<Event>,
}

impl DaemonEventGenerator {
    pub async fn new(
        guests: GuestStore,
        guest_reconciler_notify: Sender<Uuid>,
        runtime: Runtime,
    ) -> Result<(DaemonEventContext, DaemonEventGenerator)> {
        let (sender, _) = broadcast::channel(EVENT_CHANNEL_QUEUE_LEN);
        let generator = DaemonEventGenerator {
            runtime,
            guests,
            guest_reconciler_notify,
            last: HashMap::new(),
            _sender: sender.clone(),
        };
        let context = DaemonEventContext { sender };
        Ok((context, generator))
    }

    async fn evaluate(&mut self) -> Result<()> {
        let guests = self.runtime.list().await?;
        let guests = {
            let mut map = HashMap::new();
            for guest in guests {
                map.insert(guest.uuid, guest);
            }
            map
        };

        let mut exits: Vec<(Uuid, i32)> = Vec::new();

        for (uuid, guest) in &guests {
            let Some(last) = self.last.get(uuid) else {
                continue;
            };

            if last.state.exit_code.is_some() {
                continue;
            }

            let Some(code) = guest.state.exit_code else {
                continue;
            };

            exits.push((*uuid, code));
        }

        for (uuid, code) in exits {
            if let Some(mut entry) = self.guests.read(uuid).await? {
                let Some(ref mut guest) = entry.guest else {
                    continue;
                };

                guest.state = Some(GuestState {
                    status: GuestStatus::Exited.into(),
                    exit_info: Some(GuestExitInfo { code }),
                    error_info: None,
                });

                self.guests.update(uuid, entry).await?;
                self.guest_reconciler_notify.send(uuid).await?;
            }
        }

        self.last = guests;
        Ok(())
    }

    pub async fn launch(mut self) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            loop {
                if let Err(error) = self.evaluate().await {
                    error!("failed to evaluate daemon events: {}", error);
                    time::sleep(Duration::from_secs(5)).await;
                } else {
                    time::sleep(Duration::from_millis(500)).await;
                }
            }
        }))
    }
}
