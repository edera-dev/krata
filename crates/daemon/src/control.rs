use async_stream::try_stream;
use futures::Stream;
use krata::{
    idm::internal::{
        request::Request as IdmRequestType, response::Response as IdmResponseType, MetricsRequest,
        Request as IdmRequest,
    },
    v1::{
        common::{Guest, GuestState, GuestStatus, OciImageFormat},
        control::{
            control_service_server::ControlService, ConsoleDataReply, ConsoleDataRequest,
            CreateGuestReply, CreateGuestRequest, DestroyGuestReply, DestroyGuestRequest,
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
    console::DaemonConsoleHandle, db::GuestStore, event::DaemonEventContext, idm::DaemonIdmHandle,
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
    events: DaemonEventContext,
    console: DaemonConsoleHandle,
    idm: DaemonIdmHandle,
    guests: GuestStore,
    guest_reconciler_notify: Sender<Uuid>,
    packer: OciPackerService,
}

impl DaemonControlService {
    pub fn new(
        events: DaemonEventContext,
        console: DaemonConsoleHandle,
        idm: DaemonIdmHandle,
        guests: GuestStore,
        guest_reconciler_notify: Sender<Uuid>,
        packer: OciPackerService,
    ) -> Self {
        Self {
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
    type ConsoleDataStream =
        Pin<Box<dyn Stream<Item = Result<ConsoleDataReply, Status>> + Send + 'static>>;

    type PullImageStream =
        Pin<Box<dyn Stream<Item = Result<PullImageReply, Status>> + Send + 'static>>;

    type WatchEventsStream =
        Pin<Box<dyn Stream<Item = Result<WatchEventsReply, Status>> + Send + 'static>>;

    type SnoopIdmStream =
        Pin<Box<dyn Stream<Item = Result<SnoopIdmReply, Status>> + Send + 'static>>;

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
        let guest = self
            .guests
            .read(uuid)
            .await
            .map_err(|error| ApiError {
                message: error.to_string(),
            })?
            .ok_or_else(|| ApiError {
                message: "guest did not exist in the database".to_string(),
            })?;

        let Some(ref state) = guest.state else {
            return Err(ApiError {
                message: "guest did not have state".to_string(),
            }
            .into());
        };

        let domid = state.domid;
        if domid == 0 {
            return Err(ApiError {
                message: "invalid domid on the guest".to_string(),
            }
            .into());
        }

        let (sender, mut receiver) = channel(100);
        let console = self
            .console
            .attach(domid, sender)
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
        let guest = self
            .guests
            .read(uuid)
            .await
            .map_err(|error| ApiError {
                message: error.to_string(),
            })?
            .ok_or_else(|| ApiError {
                message: "guest did not exist in the database".to_string(),
            })?;

        let Some(ref state) = guest.state else {
            return Err(ApiError {
                message: "guest did not have state".to_string(),
            }
            .into());
        };

        let domid = state.domid;
        if domid == 0 {
            return Err(ApiError {
                message: "invalid domid on the guest".to_string(),
            }
            .into());
        }

        let client = self.idm.client(domid).await.map_err(|error| ApiError {
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
                                _ => OciImageFormat::Unknown.into(),
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
        let output = try_stream! {
            while let Ok(event) = messages.recv().await {
                yield SnoopIdmReply { from: event.from, to: event.to, packet: Some(event.packet) };
            }
        };
        Ok(Response::new(Box::pin(output) as Self::SnoopIdmStream))
    }
}
