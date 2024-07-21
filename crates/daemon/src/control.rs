use crate::db::zone::ZoneStore;
use crate::{
    command::DaemonCommand, console::DaemonConsoleHandle, devices::DaemonDeviceManager,
    event::DaemonEventContext, idm::DaemonIdmHandle, metrics::idm_metric_to_api,
    oci::convert_oci_progress, zlt::ZoneLookupTable,
};
use async_stream::try_stream;
use futures::Stream;
use krata::v1::control::{
    GetZoneReply, GetZoneRequest, SetHostPowerManagementPolicyReply,
    SetHostPowerManagementPolicyRequest,
};
use krata::{
    idm::internal::{
        exec_stream_request_update::Update, request::Request as IdmRequestType,
        response::Response as IdmResponseType, ExecEnvVar, ExecStreamRequestStart,
        ExecStreamRequestStdin, ExecStreamRequestUpdate, MetricsRequest, Request as IdmRequest,
    },
    v1::{
        common::{OciImageFormat, Zone, ZoneState, ZoneStatus},
        control::{
            control_service_server::ControlService, CreateZoneReply, CreateZoneRequest,
            DestroyZoneReply, DestroyZoneRequest, DeviceInfo, ExecInsideZoneReply,
            ExecInsideZoneRequest, GetHostCpuTopologyReply, GetHostCpuTopologyRequest,
            HostCpuTopologyInfo, HostStatusReply, HostStatusRequest, ListDevicesReply,
            ListDevicesRequest, ListZonesReply, ListZonesRequest, PullImageReply, PullImageRequest,
            ReadZoneMetricsReply, ReadZoneMetricsRequest, ResolveZoneIdReply, ResolveZoneIdRequest,
            SnoopIdmReply, SnoopIdmRequest, WatchEventsReply, WatchEventsRequest, ZoneConsoleReply,
            ZoneConsoleRequest,
        },
    },
};
use krataoci::{
    name::ImageName,
    packer::{service::OciPackerService, OciPackedFormat, OciPackedImage},
    progress::{OciProgress, OciProgressContext},
};
use kratart::Runtime;
use std::{pin::Pin, str::FromStr};
use tokio::{
    select,
    sync::mpsc::{channel, Sender},
    task::JoinError,
};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

pub struct ApiError {
    message: String,
}

impl From<anyhow::Error> for ApiError {
    fn from(value: anyhow::Error) -> Self {
        ApiError {
            message: value.to_string(),
        }
    }
}

impl From<ApiError> for Status {
    fn from(value: ApiError) -> Self {
        Status::unknown(value.message)
    }
}

#[derive(Clone)]
pub struct DaemonControlService {
    glt: ZoneLookupTable,
    devices: DaemonDeviceManager,
    events: DaemonEventContext,
    console: DaemonConsoleHandle,
    idm: DaemonIdmHandle,
    zones: ZoneStore,
    zone_reconciler_notify: Sender<Uuid>,
    packer: OciPackerService,
    runtime: Runtime,
}

impl DaemonControlService {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        glt: ZoneLookupTable,
        devices: DaemonDeviceManager,
        events: DaemonEventContext,
        console: DaemonConsoleHandle,
        idm: DaemonIdmHandle,
        zones: ZoneStore,
        zone_reconciler_notify: Sender<Uuid>,
        packer: OciPackerService,
        runtime: Runtime,
    ) -> Self {
        Self {
            glt,
            devices,
            events,
            console,
            idm,
            zones,
            zone_reconciler_notify,
            packer,
            runtime,
        }
    }
}

enum ConsoleDataSelect {
    Read(Option<Vec<u8>>),
    Write(Option<Result<ZoneConsoleRequest, Status>>),
}

enum PullImageSelect {
    Progress(Option<OciProgress>),
    Completed(Result<Result<OciPackedImage, anyhow::Error>, JoinError>),
}

#[tonic::async_trait]
impl ControlService for DaemonControlService {
    type ExecInsideZoneStream =
        Pin<Box<dyn Stream<Item = Result<ExecInsideZoneReply, Status>> + Send + 'static>>;

    type AttachZoneConsoleStream =
        Pin<Box<dyn Stream<Item = Result<ZoneConsoleReply, Status>> + Send + 'static>>;

    type PullImageStream =
        Pin<Box<dyn Stream<Item = Result<PullImageReply, Status>> + Send + 'static>>;

    type WatchEventsStream =
        Pin<Box<dyn Stream<Item = Result<WatchEventsReply, Status>> + Send + 'static>>;

    type SnoopIdmStream =
        Pin<Box<dyn Stream<Item = Result<SnoopIdmReply, Status>> + Send + 'static>>;

    async fn host_status(
        &self,
        request: Request<HostStatusRequest>,
    ) -> Result<Response<HostStatusReply>, Status> {
        let _ = request.into_inner();
        Ok(Response::new(HostStatusReply {
            host_domid: self.glt.host_domid(),
            host_uuid: self.glt.host_uuid().to_string(),
            krata_version: DaemonCommand::version(),
        }))
    }

    async fn create_zone(
        &self,
        request: Request<CreateZoneRequest>,
    ) -> Result<Response<CreateZoneReply>, Status> {
        let request = request.into_inner();
        let Some(spec) = request.spec else {
            return Err(ApiError {
                message: "zone spec not provided".to_string(),
            }
            .into());
        };
        let uuid = Uuid::new_v4();
        self.zones
            .update(
                uuid,
                Zone {
                    id: uuid.to_string(),
                    status: Some(ZoneStatus {
                        state: ZoneState::Creating.into(),
                        network_status: None,
                        exit_status: None,
                        error_status: None,
                        host: self.glt.host_uuid().to_string(),
                        domid: u32::MAX,
                    }),
                    spec: Some(spec),
                },
            )
            .await
            .map_err(ApiError::from)?;
        self.zone_reconciler_notify
            .send(uuid)
            .await
            .map_err(|x| ApiError {
                message: x.to_string(),
            })?;
        Ok(Response::new(CreateZoneReply {
            zone_id: uuid.to_string(),
        }))
    }

    async fn exec_inside_zone(
        &self,
        request: Request<Streaming<ExecInsideZoneRequest>>,
    ) -> Result<Response<Self::ExecInsideZoneStream>, Status> {
        let mut input = request.into_inner();
        let Some(request) = input.next().await else {
            return Err(ApiError {
                message: "expected to have at least one request".to_string(),
            }
            .into());
        };
        let request = request?;

        let Some(task) = request.task else {
            return Err(ApiError {
                message: "task is missing".to_string(),
            }
            .into());
        };

        let uuid = Uuid::from_str(&request.zone_id).map_err(|error| ApiError {
            message: error.to_string(),
        })?;
        let idm = self.idm.client(uuid).await.map_err(|error| ApiError {
            message: error.to_string(),
        })?;

        let idm_request = IdmRequest {
            request: Some(IdmRequestType::ExecStream(ExecStreamRequestUpdate {
                update: Some(Update::Start(ExecStreamRequestStart {
                    environment: task
                        .environment
                        .into_iter()
                        .map(|x| ExecEnvVar {
                            key: x.key,
                            value: x.value,
                        })
                        .collect(),
                    command: task.command,
                    working_directory: task.working_directory,
                })),
            })),
        };

        let output = try_stream! {
            let mut handle = idm.send_stream(idm_request).await.map_err(|x| ApiError {
                message: x.to_string(),
            })?;

            loop {
                select! {
                    x = input.next() => if let Some(update) = x {
                        let update: Result<ExecInsideZoneRequest, Status> = update.map_err(|error| ApiError {
                            message: error.to_string()
                        }.into());

                        if let Ok(update) = update {
                            if !update.data.is_empty() {
                                let _ = handle.update(IdmRequest {
                                    request: Some(IdmRequestType::ExecStream(ExecStreamRequestUpdate {
                                        update: Some(Update::Stdin(ExecStreamRequestStdin {
                                            data: update.data,
                                        })),
                                    }))}).await;
                            }
                        }
                    },
                    x = handle.receiver.recv() => match x {
                        Some(response) => {
                            let Some(IdmResponseType::ExecStream(update)) = response.response else {
                                break;
                            };
                            let reply = ExecInsideZoneReply {
                                exited: update.exited,
                                error: update.error,
                                exit_code: update.exit_code,
                                stdout: update.stdout,
                                stderr: update.stderr
                            };
                            yield reply;
                        },
                        None => {
                            break;
                        }
                    }
                }
            }
        };

        Ok(Response::new(Box::pin(output) as Self::ExecInsideZoneStream))
    }

    async fn destroy_zone(
        &self,
        request: Request<DestroyZoneRequest>,
    ) -> Result<Response<DestroyZoneReply>, Status> {
        let request = request.into_inner();
        let uuid = Uuid::from_str(&request.zone_id).map_err(|error| ApiError {
            message: error.to_string(),
        })?;
        let Some(mut zone) = self.zones.read(uuid).await.map_err(ApiError::from)? else {
            return Err(ApiError {
                message: "zone not found".to_string(),
            }
            .into());
        };

        zone.status = Some(zone.status.as_mut().cloned().unwrap_or_default());

        if zone.status.as_ref().unwrap().state() == ZoneState::Destroyed {
            return Err(ApiError {
                message: "zone already destroyed".to_string(),
            }
            .into());
        }

        zone.status.as_mut().unwrap().state = ZoneState::Destroying.into();
        self.zones
            .update(uuid, zone)
            .await
            .map_err(ApiError::from)?;
        self.zone_reconciler_notify
            .send(uuid)
            .await
            .map_err(|x| ApiError {
                message: x.to_string(),
            })?;
        Ok(Response::new(DestroyZoneReply {}))
    }

    async fn list_zones(
        &self,
        request: Request<ListZonesRequest>,
    ) -> Result<Response<ListZonesReply>, Status> {
        let _ = request.into_inner();
        let zones = self.zones.list().await.map_err(ApiError::from)?;
        let zones = zones.into_values().collect::<Vec<Zone>>();
        Ok(Response::new(ListZonesReply { zones }))
    }

    async fn resolve_zone_id(
        &self,
        request: Request<ResolveZoneIdRequest>,
    ) -> Result<Response<ResolveZoneIdReply>, Status> {
        let request = request.into_inner();
        let zones = self.zones.list().await.map_err(ApiError::from)?;
        let zones = zones
            .into_values()
            .filter(|x| {
                let comparison_spec = x.spec.as_ref().cloned().unwrap_or_default();
                (!request.name.is_empty() && comparison_spec.name == request.name)
                    || x.id == request.name
            })
            .collect::<Vec<Zone>>();
        Ok(Response::new(ResolveZoneIdReply {
            zone_id: zones.first().cloned().map(|x| x.id).unwrap_or_default(),
        }))
    }

    async fn attach_zone_console(
        &self,
        request: Request<Streaming<ZoneConsoleRequest>>,
    ) -> Result<Response<Self::AttachZoneConsoleStream>, Status> {
        let mut input = request.into_inner();
        let Some(request) = input.next().await else {
            return Err(ApiError {
                message: "expected to have at least one request".to_string(),
            }
            .into());
        };
        let request = request?;
        let uuid = Uuid::from_str(&request.zone_id).map_err(|error| ApiError {
            message: error.to_string(),
        })?;
        let (sender, mut receiver) = channel(100);
        let console = self
            .console
            .attach(uuid, sender)
            .await
            .map_err(|error| ApiError {
                message: format!("failed to attach to console: {}", error),
            })?;

        let output = try_stream! {
            yield ZoneConsoleReply { data: console.initial.clone(), };
            loop {
                let what = select! {
                    x = receiver.recv() => ConsoleDataSelect::Read(x),
                    x = input.next() => ConsoleDataSelect::Write(x),
                };

                match what {
                    ConsoleDataSelect::Read(Some(data)) => {
                        yield ZoneConsoleReply { data, };
                    },

                    ConsoleDataSelect::Read(None) => {
                        break;
                    }

                    ConsoleDataSelect::Write(Some(request)) => {
                        let request = request?;
                        if !request.data.is_empty() {
                            console.send(request.data).await.map_err(|error| ApiError {
                                message: error.to_string(),
                            })?;
                        }
                    },

                    ConsoleDataSelect::Write(None) => {
                        break;
                    }
                }
            }
        };

        Ok(Response::new(
            Box::pin(output) as Self::AttachZoneConsoleStream
        ))
    }

    async fn read_zone_metrics(
        &self,
        request: Request<ReadZoneMetricsRequest>,
    ) -> Result<Response<ReadZoneMetricsReply>, Status> {
        let request = request.into_inner();
        let uuid = Uuid::from_str(&request.zone_id).map_err(|error| ApiError {
            message: error.to_string(),
        })?;
        let client = self.idm.client(uuid).await.map_err(|error| ApiError {
            message: error.to_string(),
        })?;

        let response = client
            .send(IdmRequest {
                request: Some(IdmRequestType::Metrics(MetricsRequest {})),
            })
            .await
            .map_err(|error| ApiError {
                message: error.to_string(),
            })?;

        let mut reply = ReadZoneMetricsReply::default();
        if let Some(IdmResponseType::Metrics(metrics)) = response.response {
            reply.root = metrics.root.map(idm_metric_to_api);
        }
        Ok(Response::new(reply))
    }

    async fn pull_image(
        &self,
        request: Request<PullImageRequest>,
    ) -> Result<Response<Self::PullImageStream>, Status> {
        let request = request.into_inner();
        let name = ImageName::parse(&request.image).map_err(|err| ApiError {
            message: err.to_string(),
        })?;
        let format = match request.format() {
            OciImageFormat::Unknown => OciPackedFormat::Squashfs,
            OciImageFormat::Squashfs => OciPackedFormat::Squashfs,
            OciImageFormat::Erofs => OciPackedFormat::Erofs,
            OciImageFormat::Tar => OciPackedFormat::Tar,
        };
        let (context, mut receiver) = OciProgressContext::create();
        let our_packer = self.packer.clone();

        let output = try_stream! {
            let mut task = tokio::task::spawn(async move {
                our_packer.request(name, format, request.overwrite_cache, request.update, context).await
            });
            let abort_handle = task.abort_handle();
            let _task_cancel_guard = scopeguard::guard(abort_handle, |handle| {
                handle.abort();
            });

            loop {
                let what = select! {
                    x = receiver.changed() => match x {
                        Ok(_) => PullImageSelect::Progress(Some(receiver.borrow_and_update().clone())),
                        Err(_) => PullImageSelect::Progress(None),
                    },
                    x = &mut task => PullImageSelect::Completed(x),
                };
                match what {
                    PullImageSelect::Progress(Some(progress)) => {
                        let reply = PullImageReply {
                            progress: Some(convert_oci_progress(progress)),
                            digest: String::new(),
                            format: OciImageFormat::Unknown.into(),
                        };
                        yield reply;
                    },

                    PullImageSelect::Completed(result) => {
                        let result = result.map_err(|err| ApiError {
                            message: err.to_string(),
                        })?;
                        let packed = result.map_err(|err| ApiError {
                            message: err.to_string(),
                        })?;
                        let reply = PullImageReply {
                            progress: None,
                            digest: packed.digest,
                            format: match packed.format {
                                OciPackedFormat::Squashfs => OciImageFormat::Squashfs.into(),
                                OciPackedFormat::Erofs => OciImageFormat::Erofs.into(),
                                OciPackedFormat::Tar => OciImageFormat::Tar.into(),
                            },
                        };
                        yield reply;
                        break;
                    },

                    _ => {
                        continue;
                    }
                }
            }
        };
        Ok(Response::new(Box::pin(output) as Self::PullImageStream))
    }

    async fn watch_events(
        &self,
        request: Request<WatchEventsRequest>,
    ) -> Result<Response<Self::WatchEventsStream>, Status> {
        let _ = request.into_inner();
        let mut events = self.events.subscribe();
        let output = try_stream! {
            while let Ok(event) = events.recv().await {
                yield WatchEventsReply { event: Some(event), };
            }
        };
        Ok(Response::new(Box::pin(output) as Self::WatchEventsStream))
    }

    async fn snoop_idm(
        &self,
        request: Request<SnoopIdmRequest>,
    ) -> Result<Response<Self::SnoopIdmStream>, Status> {
        let _ = request.into_inner();
        let mut messages = self.idm.snoop();
        let glt = self.glt.clone();
        let output = try_stream! {
            while let Ok(event) = messages.recv().await {
                let Some(from_uuid) = glt.lookup_uuid_by_domid(event.from).await else {
                    continue;
                };
                let Some(to_uuid) = glt.lookup_uuid_by_domid(event.to).await else {
                    continue;
                };
                yield SnoopIdmReply { from: from_uuid.to_string(), to: to_uuid.to_string(), packet: Some(event.packet) };
            }
        };
        Ok(Response::new(Box::pin(output) as Self::SnoopIdmStream))
    }

    async fn list_devices(
        &self,
        request: Request<ListDevicesRequest>,
    ) -> Result<Response<ListDevicesReply>, Status> {
        let _ = request.into_inner();
        let mut devices = Vec::new();
        let state = self.devices.copy().await.map_err(|error| ApiError {
            message: error.to_string(),
        })?;
        for (name, state) in state {
            devices.push(DeviceInfo {
                name,
                claimed: state.owner.is_some(),
                owner: state.owner.map(|x| x.to_string()).unwrap_or_default(),
            });
        }
        Ok(Response::new(ListDevicesReply { devices }))
    }

    async fn get_host_cpu_topology(
        &self,
        request: Request<GetHostCpuTopologyRequest>,
    ) -> Result<Response<GetHostCpuTopologyReply>, Status> {
        let _ = request.into_inner();
        let power = self
            .runtime
            .power_management_context()
            .await
            .map_err(ApiError::from)?;
        let cputopo = power.cpu_topology().await.map_err(ApiError::from)?;
        let mut cpus = vec![];

        for cpu in cputopo {
            cpus.push(HostCpuTopologyInfo {
                core: cpu.core,
                socket: cpu.socket,
                node: cpu.node,
                thread: cpu.thread,
                class: cpu.class as i32,
            })
        }

        Ok(Response::new(GetHostCpuTopologyReply { cpus }))
    }

    async fn set_host_power_management_policy(
        &self,
        request: Request<SetHostPowerManagementPolicyRequest>,
    ) -> Result<Response<SetHostPowerManagementPolicyReply>, Status> {
        let policy = request.into_inner();
        let power = self
            .runtime
            .power_management_context()
            .await
            .map_err(ApiError::from)?;
        let scheduler = &policy.scheduler;

        power
            .set_smt_policy(policy.smt_awareness)
            .await
            .map_err(ApiError::from)?;
        power
            .set_scheduler_policy(scheduler)
            .await
            .map_err(ApiError::from)?;

        Ok(Response::new(SetHostPowerManagementPolicyReply {}))
    }

    async fn get_zone(
        &self,
        request: Request<GetZoneRequest>,
    ) -> Result<Response<GetZoneReply>, Status> {
        let request = request.into_inner();
        let zones = self.zones.list().await.map_err(ApiError::from)?;
        let zone = zones.get(&Uuid::from_str(&request.zone_id).map_err(|error| ApiError {
            message: error.to_string(),
        })?);
        Ok(Response::new(GetZoneReply {
            zone: zone.cloned(),
        }))
    }
}
