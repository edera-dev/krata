use std::{
    collections::{hash_map::Entry, HashMap},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use anyhow::Result;
use krata::v1::{
    common::{Zone, ZoneErrorInfo, ZoneExitInfo, ZoneNetworkState, ZoneState, ZoneStatus},
    control::ZoneChangedEvent,
};
use krataoci::packer::service::OciPackerService;
use kratart::{Runtime, ZoneInfo};
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

use crate::{
    db::ZoneStore,
    devices::DaemonDeviceManager,
    event::{DaemonEvent, DaemonEventContext},
    zlt::ZoneLookupTable,
};

use self::start::ZoneStarter;

mod start;

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
                };
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
                    let mut state = stored_zone.state.as_mut().cloned().unwrap_or_default();
                    if state.status() == ZoneStatus::Started {
                        state.status = ZoneStatus::Starting.into();
                    }
                    stored_zone.state = Some(state);
                }

                Some(runtime) => {
                    self.zlt.associate(uuid, runtime.domid).await;
                    let mut state = stored_zone.state.as_mut().cloned().unwrap_or_default();
                    if let Some(code) = runtime.state.exit_code {
                        state.status = ZoneStatus::Exited.into();
                        state.exit_info = Some(ZoneExitInfo { code });
                    } else {
                        state.status = ZoneStatus::Started.into();
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

                    state.network = Some(zoneinfo_to_networkstate(runtime));
                    stored_zone.state = Some(state);
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

        let start_status = zone.state.as_ref().map(|x| x.status()).unwrap_or_default();
        let result = match start_status {
            ZoneStatus::Starting => self.start(uuid, &mut zone).await,
            ZoneStatus::Exited => self.exited(&mut zone).await,
            ZoneStatus::Destroying => self.destroy(uuid, &mut zone).await,
            _ => Ok(ZoneReconcilerResult::Unchanged),
        };

        let result = match result {
            Ok(result) => result,
            Err(error) => {
                zone.state = Some(zone.state.as_mut().cloned().unwrap_or_default());
                zone.state.as_mut().unwrap().status = ZoneStatus::Failed.into();
                zone.state.as_mut().unwrap().error_info = Some(ZoneErrorInfo {
                    message: error.to_string(),
                });
                warn!("failed to start zone {}: {}", zone.id, error);
                ZoneReconcilerResult::Changed { rerun: false }
            }
        };

        info!("reconciled zone {}", uuid);

        let status = zone.state.as_ref().map(|x| x.status()).unwrap_or_default();
        let destroyed = status == ZoneStatus::Destroyed;

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

    async fn start(&self, uuid: Uuid, zone: &mut Zone) -> Result<ZoneReconcilerResult> {
        let starter = ZoneStarter {
            devices: &self.devices,
            kernel_path: &self.kernel_path,
            initrd_path: &self.initrd_path,
            addons_path: &self.addons_path,
            packer: &self.packer,
            glt: &self.zlt,
            runtime: &self.runtime,
        };
        starter.start(uuid, zone).await
    }

    async fn exited(&self, zone: &mut Zone) -> Result<ZoneReconcilerResult> {
        if let Some(ref mut state) = zone.state {
            state.set_status(ZoneStatus::Destroying);
            Ok(ZoneReconcilerResult::Changed { rerun: true })
        } else {
            Ok(ZoneReconcilerResult::Unchanged)
        }
    }

    async fn destroy(&self, uuid: Uuid, zone: &mut Zone) -> Result<ZoneReconcilerResult> {
        if let Err(error) = self.runtime.destroy(uuid).await {
            trace!("failed to destroy runtime zone {}: {}", uuid, error);
        }

        let domid = zone.state.as_ref().map(|x| x.domid);

        if let Some(domid) = domid {
            self.zlt.remove(uuid, domid).await;
        }

        info!("destroyed zone {}", uuid);
        zone.state = Some(ZoneState {
            status: ZoneStatus::Destroyed.into(),
            network: None,
            exit_info: None,
            error_info: None,
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

pub fn zoneinfo_to_networkstate(info: &ZoneInfo) -> ZoneNetworkState {
    ZoneNetworkState {
        zone_ipv4: info.zone_ipv4.map(|x| x.to_string()).unwrap_or_default(),
        zone_ipv6: info.zone_ipv6.map(|x| x.to_string()).unwrap_or_default(),
        zone_mac: info.zone_mac.as_ref().cloned().unwrap_or_default(),
        gateway_ipv4: info.gateway_ipv4.map(|x| x.to_string()).unwrap_or_default(),
        gateway_ipv6: info.gateway_ipv6.map(|x| x.to_string()).unwrap_or_default(),
        gateway_mac: info.gateway_mac.as_ref().cloned().unwrap_or_default(),
    }
}
