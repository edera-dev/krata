use std::{
    collections::{hash_map::Entry, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use self::create::ZoneCreator;
use crate::config::DaemonConfig;
use crate::db::ip::IpReservation;
use crate::ip::assignment::IpAssignment;
use crate::{
    db::zone::ZoneStore,
    devices::DaemonDeviceManager,
    event::{DaemonEvent, DaemonEventContext},
    zlt::ZoneLookupTable,
};
use anyhow::Result;
use krata::v1::{
    common::{Zone, ZoneErrorStatus, ZoneExitStatus, ZoneNetworkStatus, ZoneState, ZoneStatus},
    control::ZoneChangedEvent,
};
use krataoci::packer::service::OciPackerService;
use kratart::Runtime;
use log::{error, info, trace, warn};
use tokio::{
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        Mutex, RwLock,
    },
    task::JoinHandle,
    time::sleep,
};
use uuid::Uuid;

mod create;

const PARALLEL_LIMIT: u32 = 5;

#[derive(Debug)]
enum ZoneReconcilerResult {
    Unchanged,
    Changed { rerun: bool },
}

struct ZoneReconcilerEntry {
    task: JoinHandle<()>,
    sender: Sender<()>,
}

impl Drop for ZoneReconcilerEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

#[derive(Clone)]
pub struct ZoneReconciler {
    devices: DaemonDeviceManager,
    zlt: ZoneLookupTable,
    zones: ZoneStore,
    events: DaemonEventContext,
    runtime: Runtime,
    packer: OciPackerService,
    kernel_path: PathBuf,
    initrd_path: PathBuf,
    addons_path: PathBuf,
    tasks: Arc<Mutex<HashMap<Uuid, ZoneReconcilerEntry>>>,
    zone_reconciler_notify: Sender<Uuid>,
    zone_reconcile_lock: Arc<RwLock<()>>,
    ip_assignment: IpAssignment,
    config: Arc<DaemonConfig>,
}

impl ZoneReconciler {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        devices: DaemonDeviceManager,
        zlt: ZoneLookupTable,
        zones: ZoneStore,
        events: DaemonEventContext,
        runtime: Runtime,
        packer: OciPackerService,
        zone_reconciler_notify: Sender<Uuid>,
        kernel_path: PathBuf,
        initrd_path: PathBuf,
        modules_path: PathBuf,
        ip_assignment: IpAssignment,
        config: Arc<DaemonConfig>,
    ) -> Result<Self> {
        Ok(Self {
            devices,
            zlt,
            zones,
            events,
            runtime,
            packer,
            kernel_path,
            initrd_path,
            addons_path: modules_path,
            tasks: Arc::new(Mutex::new(HashMap::new())),
            zone_reconciler_notify,
            zone_reconcile_lock: Arc::new(RwLock::with_max_readers((), PARALLEL_LIMIT)),
            ip_assignment,
            config,
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
                                error!("failed to start zone reconciler task {}: {}", uuid, error);
                            }

                            let map = self.tasks.lock().await;
                            if let Some(entry) = map.get(&uuid) {
                                if let Err(error) = entry.sender.send(()).await {
                                    error!("failed to notify zone reconciler task {}: {}", uuid, error);
                                }
                            }
                        }
                    },

                    _ = sleep(Duration::from_secs(15)) => {
                        if let Err(error) = self.reconcile_runtime(false).await {
                            error!("runtime reconciler failed: {}", error);
                        }
                    }
                }
            }
        }))
    }

    pub async fn reconcile_runtime(&self, initial: bool) -> Result<()> {
        let _permit = self.zone_reconcile_lock.write().await;
        trace!("reconciling runtime");
        let runtime_zones = self.runtime.list().await?;
        let stored_zones = self.zones.list().await?;

        let non_existent_zones = runtime_zones
            .iter()
            .filter(|x| !stored_zones.iter().any(|g| *g.0 == x.uuid))
            .collect::<Vec<_>>();

        for zone in non_existent_zones {
            warn!("destroying unknown runtime zone {}", zone.uuid);
            if let Err(error) = self.runtime.destroy(zone.uuid).await {
                error!(
                    "failed to destroy unknown runtime zone {}: {}",
                    zone.uuid, error
                );
            }
            self.zones.remove(zone.uuid).await?;
        }

        let mut device_claims = HashMap::new();

        for (uuid, mut stored_zone) in stored_zones {
            let previous_zone = stored_zone.clone();
            let runtime_zone = runtime_zones.iter().find(|x| x.uuid == uuid);
            match runtime_zone {
                None => {
                    let mut status = stored_zone.status.as_mut().cloned().unwrap_or_default();
                    if status.state() == ZoneState::Created {
                        status.state = ZoneState::Creating.into();
                    }
                    stored_zone.status = Some(status);
                }

                Some(runtime) => {
                    self.zlt.associate(uuid, runtime.domid).await;
                    let mut status = stored_zone.status.as_mut().cloned().unwrap_or_default();
                    if let Some(code) = runtime.state.exit_code {
                        status.state = ZoneState::Exited.into();
                        status.exit_status = Some(ZoneExitStatus { code });
                    } else {
                        status.state = ZoneState::Created.into();
                    }

                    for device in &stored_zone
                        .spec
                        .as_ref()
                        .cloned()
                        .unwrap_or_default()
                        .devices
                    {
                        device_claims.insert(device.name.clone(), uuid);
                    }

                    if let Some(reservation) = self.ip_assignment.retrieve(uuid).await? {
                        status.network_status =
                            Some(ip_reservation_to_network_status(&reservation));
                    }
                    stored_zone.status = Some(status);
                }
            }

            let changed = stored_zone != previous_zone;

            if changed || initial {
                self.zones.update(uuid, stored_zone).await?;
                let _ = self.zone_reconciler_notify.try_send(uuid);
            }
        }

        self.devices.update_claims(device_claims).await?;

        Ok(())
    }

    pub async fn reconcile(&self, uuid: Uuid) -> Result<bool> {
        let _runtime_reconcile_permit = self.zone_reconcile_lock.read().await;
        let Some(mut zone) = self.zones.read(uuid).await? else {
            warn!(
                "notified of reconcile for zone {} but it didn't exist",
                uuid
            );
            return Ok(false);
        };

        info!("reconciling zone {}", uuid);

        self.events
            .send(DaemonEvent::ZoneChanged(ZoneChangedEvent {
                zone: Some(zone.clone()),
            }))?;

        let start_state = zone.status.as_ref().map(|x| x.state()).unwrap_or_default();
        let result = match start_state {
            ZoneState::Creating => self.create(uuid, &mut zone).await,
            ZoneState::Exited => self.exited(&mut zone).await,
            ZoneState::Destroying => self.destroy(uuid, &mut zone).await,
            _ => Ok(ZoneReconcilerResult::Unchanged),
        };

        let result = match result {
            Ok(result) => result,
            Err(error) => {
                zone.status = Some(zone.status.as_mut().cloned().unwrap_or_default());
                zone.status.as_mut().unwrap().state = ZoneState::Failed.into();
                zone.status.as_mut().unwrap().error_status = Some(ZoneErrorStatus {
                    message: error.to_string(),
                });
                warn!("failed to start zone {}: {}", zone.id, error);
                ZoneReconcilerResult::Changed { rerun: false }
            }
        };

        info!("reconciled zone {}", uuid);

        let state = zone.status.as_ref().map(|x| x.state()).unwrap_or_default();
        let destroyed = state == ZoneState::Destroyed;

        let rerun = if let ZoneReconcilerResult::Changed { rerun } = result {
            let event = DaemonEvent::ZoneChanged(ZoneChangedEvent {
                zone: Some(zone.clone()),
            });

            if destroyed {
                self.zones.remove(uuid).await?;
                let mut map = self.tasks.lock().await;
                map.remove(&uuid);
            } else {
                self.zones.update(uuid, zone.clone()).await?;
            }

            self.events.send(event)?;
            rerun
        } else {
            false
        };

        Ok(rerun)
    }

    async fn create(&self, uuid: Uuid, zone: &mut Zone) -> Result<ZoneReconcilerResult> {
        let starter = ZoneCreator {
            devices: &self.devices,
            kernel_path: &self.kernel_path,
            initrd_path: &self.initrd_path,
            addons_path: &self.addons_path,
            packer: &self.packer,
            ip_assignment: &self.ip_assignment,
            zlt: &self.zlt,
            runtime: &self.runtime,
            config: &self.config,
        };
        starter.create(uuid, zone).await
    }

    async fn exited(&self, zone: &mut Zone) -> Result<ZoneReconcilerResult> {
        if let Some(ref mut status) = zone.status {
            status.set_state(ZoneState::Destroying);
            Ok(ZoneReconcilerResult::Changed { rerun: true })
        } else {
            Ok(ZoneReconcilerResult::Unchanged)
        }
    }

    async fn destroy(&self, uuid: Uuid, zone: &mut Zone) -> Result<ZoneReconcilerResult> {
        if let Err(error) = self.runtime.destroy(uuid).await {
            trace!("failed to destroy runtime zone {}: {}", uuid, error);
        }

        let domid = zone.status.as_ref().map(|x| x.domid);

        if let Some(domid) = domid {
            self.zlt.remove(uuid, domid).await;
        }

        info!("destroyed zone {}", uuid);
        self.ip_assignment.recall(uuid).await?;
        zone.status = Some(ZoneStatus {
            state: ZoneState::Destroyed.into(),
            network_status: None,
            exit_status: None,
            error_status: None,
            resource_status: None,
            host: self.zlt.host_uuid().to_string(),
            domid: domid.unwrap_or(u32::MAX),
        });
        self.devices.release_all(uuid).await?;
        Ok(ZoneReconcilerResult::Changed { rerun: false })
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

    async fn launch_task(&self, uuid: Uuid) -> Result<ZoneReconcilerEntry> {
        let this = self.clone();
        let (sender, mut receiver) = channel(10);
        let task = tokio::task::spawn(async move {
            'notify_loop: loop {
                if receiver.recv().await.is_none() {
                    break 'notify_loop;
                }

                'rerun_loop: loop {
                    let rerun = match this.reconcile(uuid).await {
                        Ok(rerun) => rerun,
                        Err(error) => {
                            error!("failed to reconcile zone {}: {}", uuid, error);
                            false
                        }
                    };

                    if rerun {
                        continue 'rerun_loop;
                    }
                    break 'rerun_loop;
                }
            }
        });
        Ok(ZoneReconcilerEntry { task, sender })
    }
}

pub fn ip_reservation_to_network_status(ip: &IpReservation) -> ZoneNetworkStatus {
    ZoneNetworkStatus {
        zone_ipv4: format!("{}/{}", ip.ipv4, ip.ipv4_prefix),
        zone_ipv6: format!("{}/{}", ip.ipv6, ip.ipv6_prefix),
        zone_mac: ip.mac.to_string().to_lowercase().replace('-', ":"),
        gateway_ipv4: format!("{}/{}", ip.gateway_ipv4, ip.ipv4_prefix),
        gateway_ipv6: format!("{}/{}", ip.gateway_ipv6, ip.ipv6_prefix),
        gateway_mac: ip.gateway_mac.to_string().to_lowercase().replace('-', ":"),
    }
}
