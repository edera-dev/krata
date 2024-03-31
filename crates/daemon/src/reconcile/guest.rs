use std::{
    collections::{hash_map::Entry, HashMap},
    sync::Arc,
    time::Duration,
};

use anyhow::{anyhow, Result};
use krata::v1::{
    common::{
        guest_image_spec::Image, Guest, GuestErrorInfo, GuestExitInfo, GuestNetworkState,
        GuestState, GuestStatus,
    },
    control::GuestChangedEvent,
};
use kratart::{launch::GuestLaunchRequest, GuestInfo, Runtime};
use log::{error, info, trace, warn};
use tokio::{
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex, Semaphore,
    },
    task::JoinHandle,
    time::sleep,
};
use uuid::Uuid;

use crate::{
    db::GuestStore,
    event::{DaemonEvent, DaemonEventContext},
};

struct GuestReconcilerEntry {
    task: JoinHandle<()>,
    sender: Sender<()>,
}

impl Drop for GuestReconcilerEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
pub struct GuestReconciler {
    guests: GuestStore,
    events: DaemonEventContext,
    runtime: Runtime,
    limiter: Arc<Semaphore>,
    tasks: Arc<Mutex<HashMap<Uuid, GuestReconcilerEntry>>>,
}

impl GuestReconciler {
    pub fn new(guests: GuestStore, events: DaemonEventContext, runtime: Runtime) -> Result<Self> {
        Ok(Self {
            guests,
            events,
            runtime,
            limiter: Arc::new(Semaphore::new(10)),
            tasks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn launch(self, mut notify: Receiver<Uuid>) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            if let Err(error) = self.reconcile_runtime(true).await {
                error!("runtime reconciler failed: {}", error);
            }

            loop {
                select! {
                    x = notify.recv() => match x {
                        None => {
                            break;
                        },

                        Some(uuid) => {
                            if let Err(error) = self.launch_task_if_needed(uuid).await {
                                error!("failed to start guest reconciler task {}: {}", uuid, error);
                            }

                            let map = self.tasks.lock().await;
                            if let Some(entry) = map.get(&uuid) {
                                if let Err(error) = entry.sender.send(()).await {
                                    error!("failed to notify guest reconciler task {}: {}", uuid, error);
                                }
                            }
                        }
                    },

                    _ = sleep(Duration::from_secs(5)) => {
                        if let Err(error) = self.reconcile_runtime(false).await {
                            error!("runtime reconciler failed: {}", error);
                        }
                    }
                };
            }
        }))
    }

    pub async fn reconcile_runtime(&self, initial: bool) -> Result<()> {
        trace!("reconciling runtime");
        let runtime_guests = self.runtime.list().await?;
        let stored_guests = self.guests.list().await?;
        for (uuid, mut stored_guest) in stored_guests {
            let previous_guest = stored_guest.clone();
            let runtime_guest = runtime_guests.iter().find(|x| x.uuid == uuid);
            match runtime_guest {
                None => {
                    let mut state = stored_guest.state.as_mut().cloned().unwrap_or_default();
                    if state.status() == GuestStatus::Started {
                        state.status = GuestStatus::Starting.into();
                    }
                    stored_guest.state = Some(state);
                }

                Some(runtime) => {
                    let mut state = stored_guest.state.as_mut().cloned().unwrap_or_default();
                    if let Some(code) = runtime.state.exit_code {
                        state.status = GuestStatus::Exited.into();
                        state.exit_info = Some(GuestExitInfo { code });
                    } else {
                        state.status = GuestStatus::Started.into();
                    }
                    state.network = Some(guestinfo_to_networkstate(runtime));
                    stored_guest.state = Some(state);
                }
            }

            let changed = stored_guest != previous_guest;

            if changed || initial {
                self.guests.update(uuid, stored_guest).await?;
                if let Err(error) = self.reconcile(uuid).await {
                    error!("failed to reconcile guest {}: {}", uuid, error);
                }
            }
        }
        Ok(())
    }

    pub async fn reconcile(&self, uuid: Uuid) -> Result<()> {
        let _permit = self.limiter.acquire().await?;
        let Some(mut guest) = self.guests.read(uuid).await? else {
            warn!(
                "notified of reconcile for guest {} but it didn't exist",
                uuid
            );
            return Ok(());
        };

        info!("reconciling guest {}", uuid);

        self.events
            .send(DaemonEvent::GuestChanged(GuestChangedEvent {
                guest: Some(guest.clone()),
            }))?;

        let result = match guest.state.as_ref().map(|x| x.status()).unwrap_or_default() {
            GuestStatus::Starting => self.start(uuid, &mut guest).await,
            GuestStatus::Destroying | GuestStatus::Exited => self.destroy(uuid, &mut guest).await,
            _ => Ok(false),
        };

        let changed = match result {
            Ok(changed) => changed,
            Err(error) => {
                guest.state = Some(guest.state.as_mut().cloned().unwrap_or_default());
                guest.state.as_mut().unwrap().status = GuestStatus::Failed.into();
                guest.state.as_mut().unwrap().error_info = Some(GuestErrorInfo {
                    message: error.to_string(),
                });
                warn!("failed to start guest {}: {}", guest.id, error);
                true
            }
        };

        info!("reconciled guest {}", uuid);

        let status = guest.state.as_ref().map(|x| x.status()).unwrap_or_default();
        let destroyed = status == GuestStatus::Destroyed || status == GuestStatus::Failed;

        if changed {
            let event = DaemonEvent::GuestChanged(GuestChangedEvent {
                guest: Some(guest.clone()),
            });

            if destroyed {
                self.guests.remove(uuid).await?;
                let mut map = self.tasks.lock().await;
                map.remove(&uuid);
            } else {
                self.guests.update(uuid, guest.clone()).await?;
            }

            self.events.send(event)?;
        }

        Ok(())
    }

    async fn start(&self, uuid: Uuid, guest: &mut Guest) -> Result<bool> {
        let Some(ref spec) = guest.spec else {
            return Err(anyhow!("guest spec not specified"));
        };

        let Some(ref image) = spec.image else {
            return Err(anyhow!("image spec not provided"));
        };
        let oci = match image.image {
            Some(Image::Oci(ref oci)) => oci,
            None => {
                return Err(anyhow!("oci spec not specified"));
            }
        };

        let task = spec.task.as_ref().cloned().unwrap_or_default();

        let info = self
            .runtime
            .launch(GuestLaunchRequest {
                uuid: Some(uuid),
                name: if spec.name.is_empty() {
                    None
                } else {
                    Some(&spec.name)
                },
                image: &oci.image,
                vcpus: spec.vcpus,
                mem: spec.mem,
                env: task
                    .environment
                    .iter()
                    .map(|x| (x.key.clone(), x.value.clone()))
                    .collect::<HashMap<_, _>>(),
                run: empty_vec_optional(task.command.clone()),
                debug: false,
            })
            .await?;
        info!("started guest {}", uuid);
        guest.state = Some(GuestState {
            status: GuestStatus::Started.into(),
            network: Some(guestinfo_to_networkstate(&info)),
            exit_info: None,
            error_info: None,
            domid: info.domid,
        });
        Ok(true)
    }

    async fn destroy(&self, uuid: Uuid, guest: &mut Guest) -> Result<bool> {
        if let Err(error) = self.runtime.destroy(uuid).await {
            trace!("failed to destroy runtime guest {}: {}", uuid, error);
        }

        info!("destroyed guest {}", uuid);
        guest.state = Some(GuestState {
            status: GuestStatus::Destroyed.into(),
            network: None,
            exit_info: None,
            error_info: None,
            domid: guest.state.as_ref().map(|x| x.domid).unwrap_or(u32::MAX),
        });
        Ok(true)
    }

    async fn launch_task_if_needed(&self, uuid: Uuid) -> Result<()> {
        let mut map = self.tasks.lock().await;
        match map.entry(uuid) {
            Entry::Occupied(_) => {}
            Entry::Vacant(entry) => {
                entry.insert(self.launch_task(uuid).await?);
            }
        }
        Ok(())
    }

    async fn launch_task(&self, uuid: Uuid) -> Result<GuestReconcilerEntry> {
        let this = self.clone();
        let (sender, mut receiver) = channel(10);
        let task = tokio::task::spawn(async move {
            while receiver.recv().await.is_some() {
                if let Err(error) = this.reconcile(uuid).await {
                    error!("failed to reconcile guest {}: {}", uuid, error);
                }
            }
        });
        Ok(GuestReconcilerEntry { task, sender })
    }
}

fn empty_vec_optional<T>(value: Vec<T>) -> Option<Vec<T>> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn guestinfo_to_networkstate(info: &GuestInfo) -> GuestNetworkState {
    GuestNetworkState {
        guest_ipv4: info.guest_ipv4.map(|x| x.to_string()).unwrap_or_default(),
        guest_ipv6: info.guest_ipv6.map(|x| x.to_string()).unwrap_or_default(),
        guest_mac: info.guest_mac.as_ref().cloned().unwrap_or_default(),
        gateway_ipv4: info.gateway_ipv4.map(|x| x.to_string()).unwrap_or_default(),
        gateway_ipv6: info.gateway_ipv6.map(|x| x.to_string()).unwrap_or_default(),
        gateway_mac: info.gateway_mac.as_ref().cloned().unwrap_or_default(),
    }
}