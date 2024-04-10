use std::{
    collections::{hash_map::Entry, HashMap},
    str::FromStr,
    time::Duration,
};

use anyhow::Result;
use krata::{
    idm::protocol::{idm_event::Event, idm_packet::Content, IdmPacket},
    v1::common::{GuestExitInfo, GuestState, GuestStatus},
};
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

use crate::{
    db::GuestStore,
    idm::{DaemonIdmHandle, DaemonIdmSubscribeHandle},
};

pub type DaemonEvent = krata::v1::control::watch_events_reply::Event;

const EVENT_CHANNEL_QUEUE_LEN: usize = 1000;
const IDM_CHANNEL_QUEUE_LEN: usize = 1000;

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
    guests: GuestStore,
    guest_reconciler_notify: Sender<Uuid>,
    feed: broadcast::Receiver<DaemonEvent>,
    idm: DaemonIdmHandle,
    idms: HashMap<u32, (Uuid, DaemonIdmSubscribeHandle)>,
    idm_sender: Sender<(u32, IdmPacket)>,
    idm_receiver: Receiver<(u32, IdmPacket)>,
    _event_sender: broadcast::Sender<DaemonEvent>,
}

impl DaemonEventGenerator {
    pub async fn new(
        guests: GuestStore,
        guest_reconciler_notify: Sender<Uuid>,
        idm: DaemonIdmHandle,
    ) -> Result<(DaemonEventContext, DaemonEventGenerator)> {
        let (sender, _) = broadcast::channel(EVENT_CHANNEL_QUEUE_LEN);
        let (idm_sender, idm_receiver) = channel(IDM_CHANNEL_QUEUE_LEN);
        let generator = DaemonEventGenerator {
            guests,
            guest_reconciler_notify,
            feed: sender.subscribe(),
            idm,
            idms: HashMap::new(),
            idm_sender,
            idm_receiver,
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
                let domid = state.domid;
                match status {
                    GuestStatus::Started => {
                        if let Entry::Vacant(e) = self.idms.entry(domid) {
                            let subscribe =
                                self.idm.subscribe(domid, self.idm_sender.clone()).await?;
                            e.insert((id, subscribe));
                        }
                    }

                    GuestStatus::Destroyed => {
                        if let Some((_, handle)) = self.idms.remove(&domid) {
                            handle.unsubscribe().await?;
                        }
                    }

                    _ => {}
                }
            }
        }
        Ok(())
    }

    async fn handle_idm_packet(&mut self, id: Uuid, packet: IdmPacket) -> Result<()> {
        match packet.content {
            Some(Content::Event(event)) => match event.event {
                Some(Event::Exit(exit)) => self.handle_exit_code(id, exit.code).await,
                None => Ok(()),
            },

            _ => Ok(()),
        }
    }

    async fn handle_exit_code(&mut self, id: Uuid, code: i32) -> Result<()> {
        if let Some(mut guest) = self.guests.read(id).await? {
            guest.state = Some(GuestState {
                status: GuestStatus::Exited.into(),
                network: guest.state.clone().unwrap_or_default().network,
                exit_info: Some(GuestExitInfo { code }),
                error_info: None,
                domid: guest.state.clone().map(|x| x.domid).unwrap_or(u32::MAX),
            });

            self.guests.update(id, guest).await?;
            self.guest_reconciler_notify.send(id).await?;
        }
        Ok(())
    }

    async fn evaluate(&mut self) -> Result<()> {
        select! {
            x = self.idm_receiver.recv() => match x {
                Some((domid, packet)) => {
                    if let Some((id, _)) = self.idms.get(&domid) {
                        self.handle_idm_packet(*id, packet).await?;
                    }
                    Ok(())
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
