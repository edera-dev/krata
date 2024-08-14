use std::{
    collections::{hash_map::Entry, HashMap},
    str::FromStr,
    time::Duration,
};

use crate::db::zone::ZoneStore;
use crate::idm::DaemonIdmHandle;
use anyhow::Result;
use krata::v1::common::ZoneExitStatus;
use krata::{
    idm::{internal::event::Event as EventType, internal::Event},
    v1::common::{ZoneState, ZoneStatus},
};
use log::{error, warn};
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

pub type DaemonEvent = krata::v1::control::watch_events_reply::Event;

const EVENT_CHANNEL_QUEUE_LEN: usize = 1000;
const IDM_EVENT_CHANNEL_QUEUE_LEN: usize = 1000;

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
    zones: ZoneStore,
    zone_reconciler_notify: Sender<Uuid>,
    feed: broadcast::Receiver<DaemonEvent>,
    idm: DaemonIdmHandle,
    idms: HashMap<u32, (Uuid, JoinHandle<()>)>,
    idm_sender: Sender<(u32, Event)>,
    idm_receiver: Receiver<(u32, Event)>,
    _event_sender: broadcast::Sender<DaemonEvent>,
}

impl DaemonEventGenerator {
    pub async fn new(
        zones: ZoneStore,
        zone_reconciler_notify: Sender<Uuid>,
        idm: DaemonIdmHandle,
    ) -> Result<(DaemonEventContext, DaemonEventGenerator)> {
        let (sender, _) = broadcast::channel(EVENT_CHANNEL_QUEUE_LEN);
        let (idm_sender, idm_receiver) = channel(IDM_EVENT_CHANNEL_QUEUE_LEN);
        let generator = DaemonEventGenerator {
            zones,
            zone_reconciler_notify,
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
        let DaemonEvent::ZoneChanged(changed) = event;
        let Some(ref zone) = changed.zone else {
            return Ok(());
        };

        let Some(ref status) = zone.status else {
            return Ok(());
        };

        let state = status.state();
        let id = Uuid::from_str(&zone.id)?;
        let domid = status.domid;
        match state {
            ZoneState::Created => {
                if let Entry::Vacant(e) = self.idms.entry(domid) {
                    let client = self.idm.client_by_domid(domid).await?;
                    let mut receiver = client.subscribe().await?;
                    let sender = self.idm_sender.clone();
                    let task = tokio::task::spawn(async move {
                        loop {
                            let Ok(event) = receiver.recv().await else {
                                break;
                            };

                            if let Err(error) = sender.send((domid, event)).await {
                                warn!("unable to deliver idm event: {}", error);
                            }
                        }
                    });
                    e.insert((id, task));
                }
            }

            ZoneState::Destroyed => {
                if let Some((_, handle)) = self.idms.remove(&domid) {
                    handle.abort();
                }
            }

            _ => {}
        }
        Ok(())
    }

    async fn handle_idm_event(&mut self, id: Uuid, event: Event) -> Result<()> {
        match event.event {
            Some(EventType::Exit(exit)) => self.handle_exit_code(id, exit.code).await,
            None => Ok(()),
        }
    }

    async fn handle_exit_code(&mut self, id: Uuid, code: i32) -> Result<()> {
        if let Some(mut zone) = self.zones.read(id).await? {
            zone.status = Some(ZoneStatus {
                state: ZoneState::Exited.into(),
                network_status: zone.status.clone().unwrap_or_default().network_status,
                exit_status: Some(ZoneExitStatus { code }),
                error_status: None,
                host: zone.status.clone().map(|x| x.host).unwrap_or_default(),
                domid: zone.status.clone().map(|x| x.domid).unwrap_or(u32::MAX),
            });

            self.zones.update(id, zone).await?;
            self.zone_reconciler_notify.send(id).await?;
        }
        Ok(())
    }

    async fn evaluate(&mut self) -> Result<()> {
        select! {
            x = self.idm_receiver.recv() => match x {
                Some((domid, event)) => {
                    if let Some((id, _)) = self.idms.get(&domid) {
                        self.handle_idm_event(*id, event).await?;
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
            },
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
