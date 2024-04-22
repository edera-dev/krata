use async_stream::try_stream;
use futures::Stream;
use krata::{
    idm::internal::{
        exec_stream_request_update::Update, request::Request as IdmRequestType,
        response::Response as IdmResponseType, ExecEnvVar, ExecStreamRequestStart,
        ExecStreamRequestStdin, ExecStreamRequestUpdate, MetricsRequest, Request as IdmRequest,
    },
    v1::{
        common::{Guest, GuestState, GuestStatus, OciImageFormat},
        control::{
            control_service_server::ControlService, ConsoleDataReply, ConsoleDataRequest,
            CreateGuestReply, CreateGuestRequest, DestroyGuestReply, DestroyGuestRequest,
            ExecGuestReply, ExecGuestRequest, IdentifyHostReply, IdentifyHostRequest,
            ListGuestsReply, ListGuestsRequest, PullImageReply, PullImageRequest,
            ReadGuestMetricsReply, ReadGuestMetricsRequest, ResolveGuestReply, ResolveGuestRequest,
            SnoopIdmReply, SnoopIdmRequest, WatchEventsReply, WatchEventsRequest,
        },
    },
};
use krataoci::{
    name::ImageName,
    packer::{service::OciPackerService, OciPackedFormat, OciPackedImage},
    progress::{OciProgress, OciProgressContext},
};
use std::{pin::Pin, str::FromStr};
use tokio::{
    select,
    sync::mpsc::{channel, Sender},
    task::JoinError,
};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::{
    command::DaemonCommand, console::DaemonConsoleHandle, db::GuestStore,
    event::DaemonEventContext, glt::GuestLookupTable, idm::DaemonIdmHandle,
    metrics::idm_metric_to_api, oci::convert_oci_progress,
};

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
    glt: GuestLookupTable,
    events: DaemonEventContext,
    console: DaemonConsoleHandle,
    idm: DaemonIdmHandle,
    guests: GuestStore,
    guest_reconciler_notify: Sender<Uuid>,
    packer: OciPackerService,
}

impl DaemonControlService {
    pub fn new(
        glt: GuestLookupTable,
        events: DaemonEventContext,
        console: DaemonConsoleHandle,
        idm: DaemonIdmHandle,
        guests: GuestStore,
        guest_reconciler_notify: Sender<Uuid>,
        packer: OciPackerService,
    ) -> Self {
        Self {
            glt,
            events,
            console,
            idm,
            guests,
            guest_reconciler_notify,
            packer,
        }
    }
}

enum ConsoleDataSelect {
    Read(Option<Vec<u8>>),
    Write(Option<Result<ConsoleDataRequest, tonic::Status>>),
}

enum PullImageSelect {
    Progress(Option<OciProgress>),
    Completed(Result<Result<OciPackedImage, anyhow::Error>, JoinError>),
}

#[tonic::async_trait]
impl ControlService for DaemonControlService {
    type ExecGuestStream =
        Pin<Box<dyn Stream<Item = Result<ExecGuestReply, Status>> + Send + 'static>>;

    type ConsoleDataStream =
        Pin<Box<dyn Stream<Item = Result<ConsoleDataReply, Status>> + Send + 'static>>;

    type PullImageStream =
        Pin<Box<dyn Stream<Item = Result<PullImageReply, Status>> + Send + 'static>>;

    type WatchEventsStream =
        Pin<Box<dyn Stream<Item = Result<WatchEventsReply, Status>> + Send + 'static>>;

    type SnoopIdmStream =
        Pin<Box<dyn Stream<Item = Result<SnoopIdmReply, Status>> + Send + 'static>>;

    async fn identify_host(
        &self,
        request: Request<IdentifyHostRequest>,
    ) -> Result<Response<IdentifyHostReply>, Status> {
        let _ = request.into_inner();
        Ok(Response::new(IdentifyHostReply {
            host_domid: self.glt.host_domid(),
            host_uuid: self.glt.host_uuid().to_string(),
            krata_version: DaemonCommand::version(),
        }))
    }

    async fn create_guest(
        &self,
        request: Request<CreateGuestRequest>,
    ) -> Result<Response<CreateGuestReply>, Status> {
        let request = request.into_inner();
        let Some(spec) = request.spec else {
            return Err(ApiError {
                message: "guest spec not provided".to_string(),
            }
            .into());
        };
        let uuid = Uuid::new_v4();
        self.guests
            .update(
                uuid,
                Guest {
                    id: uuid.to_string(),
                    state: Some(GuestState {
                        status: GuestStatus::Starting.into(),
                        network: None,
                        exit_info: None,
                        error_info: None,
                        host: self.glt.host_uuid().to_string(),
                        domid: u32::MAX,
                    }),
                    spec: Some(spec),
                },
            )
            .await
            .map_err(ApiError::from)?;
        self.guest_reconciler_notify
            .send(uuid)
            .await
            .map_err(|x| ApiError {
                message: x.to_string(),
            })?;
        Ok(Response::new(CreateGuestReply {
            guest_id: uuid.to_string(),
        }))
    }

    async fn exec_guest(
        &self,
        request: Request<Streaming<ExecGuestRequest>>,
    ) -> Result<Response<Self::ExecGuestStream>, Status> {
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

        let uuid = Uuid::from_str(&request.guest_id).map_err(|error| ApiError {
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
                        let update: Result<ExecGuestRequest, Status> = update.map_err(|error| ApiError {
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
                            let reply = ExecGuestReply {
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
                };
            }
        };

        Ok(Response::new(Box::pin(output) as Self::ExecGuestStream))
    }

    async fn destroy_guest(
        &self,
        request: Request<DestroyGuestRequest>,
    ) -> Result<Response<DestroyGuestReply>, Status> {
        let request = request.into_inner();
        let uuid = Uuid::from_str(&request.guest_id).map_err(|error| ApiError {
            message: error.to_string(),
        })?;
        let Some(mut guest) = self.guests.read(uuid).await.map_err(ApiError::from)? else {
            return Err(ApiError {
                message: "guest not found".to_string(),
            }
            .into());
        };

        guest.state = Some(guest.state.as_mut().cloned().unwrap_or_default());

        if guest.state.as_ref().unwrap().status() == GuestStatus::Destroyed {
            return Err(ApiError {
                message: "guest already destroyed".to_string(),
            }
            .into());
        }

        guest.state.as_mut().unwrap().status = GuestStatus::Destroying.into();
        self.guests
            .update(uuid, guest)
            .await
            .map_err(ApiError::from)?;
        self.guest_reconciler_notify
            .send(uuid)
            .await
            .map_err(|x| ApiError {
                message: x.to_string(),
            })?;
        Ok(Response::new(DestroyGuestReply {}))
    }

    async fn list_guests(
        &self,
        request: Request<ListGuestsRequest>,
    ) -> Result<Response<ListGuestsReply>, Status> {
        let _ = request.into_inner();
        let guests = self.guests.list().await.map_err(ApiError::from)?;
        let guests = guests.into_values().collect::<Vec<Guest>>();
        Ok(Response::new(ListGuestsReply { guests }))
    }

    async fn resolve_guest(
        &self,
        request: Request<ResolveGuestRequest>,
    ) -> Result<Response<ResolveGuestReply>, Status> {
        let request = request.into_inner();
        let guests = self.guests.list().await.map_err(ApiError::from)?;
        let guests = guests
            .into_values()
            .filter(|x| {
                let comparison_spec = x.spec.as_ref().cloned().unwrap_or_default();
                (!request.name.is_empty() && comparison_spec.name == request.name)
                    || x.id == request.name
            })
            .collect::<Vec<Guest>>();
        Ok(Response::new(ResolveGuestReply {
            guest: guests.first().cloned(),
        }))
    }

    async fn console_data(
        &self,
        request: Request<Streaming<ConsoleDataRequest>>,
    ) -> Result<Response<Self::ConsoleDataStream>, Status> {
        let mut input = request.into_inner();
        let Some(request) = input.next().await else {
            return Err(ApiError {
                message: "expected to have at least one request".to_string(),
            }
            .into());
        };
        let request = request?;
        let uuid = Uuid::from_str(&request.guest_id).map_err(|error| ApiError {
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
            yield ConsoleDataReply { data: console.initial.clone(), };
            loop {
                let what = select! {
                    x = receiver.recv() => ConsoleDataSelect::Read(x),
                    x = input.next() => ConsoleDataSelect::Write(x),
                };

                match what {
                    ConsoleDataSelect::Read(Some(data)) => {
                        yield ConsoleDataReply { data, };
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

        Ok(Response::new(Box::pin(output) as Self::ConsoleDataStream))
    }

    async fn read_guest_metrics(
        &self,
        request: Request<ReadGuestMetricsRequest>,
    ) -> Result<Response<ReadGuestMetricsReply>, Status> {
        let request = request.into_inner();
        let uuid = Uuid::from_str(&request.guest_id).map_err(|error| ApiError {
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

        let mut reply = ReadGuestMetricsReply::default();
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
                our_packer.request(name, format, request.overwrite_cache, context).await
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
}
