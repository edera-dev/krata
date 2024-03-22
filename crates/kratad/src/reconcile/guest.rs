use anyhow::{anyhow, Result};
use krata::{
    common::{
        guest_image_spec::Image, Guest, GuestErrorInfo, GuestNetworkState, GuestState, GuestStatus,
    },
    control::GuestChangedEvent,
};
use kratart::{launch::GuestLaunchRequest, Runtime};
use log::{error, info, warn};
use tokio::{sync::mpsc::Receiver, task::JoinHandle};
use uuid::Uuid;

use crate::{
    db::GuestStore,
    event::{DaemonEvent, DaemonEventContext},
};

pub struct GuestReconciler {
    guests: GuestStore,
    events: DaemonEventContext,
    runtime: Runtime,
}

impl GuestReconciler {
    pub fn new(guests: GuestStore, events: DaemonEventContext, runtime: Runtime) -> Result<Self> {
        Ok(Self {
            guests,
            events,
            runtime,
        })
    }

    pub async fn launch(self, mut notify: Receiver<Uuid>) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            if let Err(error) = self.reconcile_runtime().await {
                error!("runtime reconciler failed: {}", error);
            }

            loop {
                let Some(uuid) = notify.recv().await else {
                    break;
                };
                if let Err(error) = self.reconcile(uuid).await {
                    error!("guest reconciler failed: {}", error);
                }
            }
        }))
    }

    pub async fn reconcile_runtime(&self) -> Result<()> {
        let runtime_guests = self.runtime.list().await?;
        let stored_guests = self.guests.list().await?;
        for (uuid, mut stored_guest_entry) in stored_guests {
            let Some(ref mut stored_guest) = stored_guest_entry.guest else {
                warn!("removing unpopulated guest entry for guest {}", uuid);
                self.guests.remove(uuid).await?;
                continue;
            };
            let runtime_guest = runtime_guests.iter().find(|x| x.uuid == uuid);
            match runtime_guest {
                None => {
                    let mut state = stored_guest.state.as_mut().cloned().unwrap_or_default();
                    if state.status() == GuestStatus::Started {
                        state.status = GuestStatus::Start.into();
                    }
                    stored_guest.state = Some(state);
                    stored_guest.network = None;
                    self.guests.update(uuid, stored_guest_entry).await?;
                    if let Err(error) = self.reconcile(uuid).await {
                        error!("failed to reconcile guest {}: {}", uuid, error);
                    }
                }

                Some(_) => {
                    let mut state = stored_guest.state.as_mut().cloned().unwrap_or_default();
                    state.status = GuestStatus::Started.into();
                    stored_guest.state = Some(state);
                    stored_guest.network = None;
                    self.guests.update(uuid, stored_guest_entry).await?;
                    if let Err(error) = self.reconcile(uuid).await {
                        error!("failed to reconcile guest {}: {}", uuid, error);
                    }
                }
            }
        }
        Ok(())
    }

    pub async fn reconcile(&self, uuid: Uuid) -> Result<()> {
        let Some(mut entry) = self.guests.read(uuid).await? else {
            warn!(
                "notified of reconcile for guest {} but it didn't exist",
                uuid
            );
            return Ok(());
        };

        info!("reconciling guest {}", uuid);

        let Some(ref mut guest) = entry.guest else {
            return Ok(());
        };

        self.events
            .send(DaemonEvent::GuestChanged(GuestChangedEvent {
                guest: Some(guest.clone()),
            }))?;

        let result = match guest.state.as_ref().map(|x| x.status()).unwrap_or_default() {
            GuestStatus::Start => self.start(uuid, guest).await,
            GuestStatus::Destroy | GuestStatus::Exited => self.destroy(uuid, guest).await,
            _ => Ok(false),
        };

        let changed = match result {
            Ok(changed) => changed,
            Err(error) => {
                guest.state = Some(guest.state.as_mut().cloned().unwrap_or_default());
                guest.state.as_mut().unwrap().error_info = Some(GuestErrorInfo {
                    message: error.to_string(),
                });
                true
            }
        };

        info!("reconciled guest {}", uuid);

        let destroyed =
            guest.state.as_ref().map(|x| x.status()).unwrap_or_default() == GuestStatus::Destroyed;

        if changed {
            let event = DaemonEvent::GuestChanged(GuestChangedEvent {
                guest: Some(guest.clone()),
            });

            if destroyed {
                self.guests.remove(uuid).await?;
            } else {
                self.guests.update(uuid, entry.clone()).await?;
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
                env: empty_vec_optional(spec.env.clone()),
                run: empty_vec_optional(spec.run.clone()),
                debug: false,
            })
            .await?;
        info!("started guest {}", uuid);
        guest.network = Some(GuestNetworkState {
            ipv4: info.ipv4.map(|x| x.ip().to_string()).unwrap_or_default(),
            ipv6: info.ipv6.map(|x| x.ip().to_string()).unwrap_or_default(),
        });
        guest.state = Some(GuestState {
            status: GuestStatus::Started.into(),
            exit_info: None,
            error_info: None,
        });
        Ok(true)
    }

    async fn destroy(&self, uuid: Uuid, guest: &mut Guest) -> Result<bool> {
        if let Err(error) = self.runtime.destroy(uuid).await {
            warn!("failed to destroy runtime guest {}: {}", uuid, error);
        }

        info!("destroyed guest {}", uuid);
        guest.network = None;
        guest.state = Some(GuestState {
            status: GuestStatus::Destroyed.into(),
            exit_info: None,
            error_info: None,
        });
        Ok(true)
    }
}

fn empty_vec_optional<T>(value: Vec<T>) -> Option<Vec<T>> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
