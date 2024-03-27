use std::{
    collections::{hash_map::Entry, HashMap},
    str::FromStr,
    time::Duration,
};

use anyhow::Result;
use krata::v1::common::{GuestExitInfo, GuestState, GuestStatus};
use log::error;
use tokio::{
    select,
    sync::{
        broadcast,
        mpsc::{channel, Receiver, Sender},
    },
    task::JoinHandle,
    time,
};
use uuid::Uuid;

use kratart::Runtime;

use crate::db::GuestStore;

pub type DaemonEvent = krata::v1::control::watch_events_reply::Event;

const EVENT_CHANNEL_QUEUE_LEN: usize = 1000;
const EXIT_CODE_CHANNEL_QUEUE_LEN: usize = 1000;

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
    feed: broadcast::Receiver<DaemonEvent>,
    exit_code_sender: Sender<(Uuid, i32)>,
    exit_code_receiver: Receiver<(Uuid, i32)>,
    exit_code_handles: HashMap<Uuid, JoinHandle<()>>,
    _event_sender: broadcast::Sender<DaemonEvent>,
}

impl DaemonEventGenerator {
    pub async fn new(
        guests: GuestStore,
        guest_reconciler_notify: Sender<Uuid>,
        runtime: Runtime,
    ) -> Result<(DaemonEventContext, DaemonEventGenerator)> {
        let (sender, _) = broadcast::channel(EVENT_CHANNEL_QUEUE_LEN);
        let (exit_code_sender, exit_code_receiver) = channel(EXIT_CODE_CHANNEL_QUEUE_LEN);
        let generator = DaemonEventGenerator {
            runtime,
            guests,
            guest_reconciler_notify,
            feed: sender.subscribe(),
            exit_code_receiver,
            exit_code_sender,
            exit_code_handles: HashMap::new(),
            _event_sender: sender.clone(),
        };
        let context = DaemonEventContext { sender };
        Ok((context, generator))
    }

    async fn handle_feed_event(&mut self, event: &DaemonEvent) -> Result<()> {
        match event {
            DaemonEvent::GuestChanged(changed) => {
                let Some(ref guest) = changed.guest else {
                    return Ok(());
                };

                let Some(ref state) = guest.state else {
                    return Ok(());
                };

                let status = state.status();
                let id = Uuid::from_str(&guest.id)?;
                match status {
                    GuestStatus::Started => {
                        if let Entry::Vacant(e) = self.exit_code_handles.entry(id) {
                            let handle = self
                                .runtime
                                .subscribe_exit_code(id, self.exit_code_sender.clone())
                                .await?;
                            e.insert(handle);
                        }
                    }

                    GuestStatus::Destroyed => {
                        if let Some(handle) = self.exit_code_handles.remove(&id) {
                            handle.abort();
                        }
                    }

                    _ => {}
                }
            }
        }
        Ok(())
    }

    async fn handle_exit_code(&mut self, id: Uuid, code: i32) -> Result<()> {
        if let Some(mut entry) = self.guests.read(id).await? {
            let Some(ref mut guest) = entry.guest else {
                return Ok(());
            };

            guest.state = Some(GuestState {
                status: GuestStatus::Exited.into(),
                network: guest.state.clone().unwrap_or_default().network,
                exit_info: Some(GuestExitInfo { code }),
                error_info: None,
                domid: guest.state.clone().map(|x| x.domid).unwrap_or(u32::MAX),
            });

            self.guests.update(id, entry).await?;
            self.guest_reconciler_notify.send(id).await?;
        }
        Ok(())
    }

    async fn evaluate(&mut self) -> Result<()> {
        select! {
            x = self.exit_code_receiver.recv() => match x {
                Some((uuid, code)) => {
                    self.handle_exit_code(uuid, code).await
                },
                None => {
                    Ok(())
                }
            },
            x = self.feed.recv() => match x {
                Ok(event) => {
                    self.handle_feed_event(&event).await
                },
                Err(error) => {
                    Err(error.into())
                }
            }
        }
    }

    pub async fn launch(mut self) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            loop {
                if let Err(error) = self.evaluate().await {
                    error!("failed to evaluate daemon events: {}", error);
                    time::sleep(Duration::from_secs(5)).await;
                }
            }
        }))
    }
}
