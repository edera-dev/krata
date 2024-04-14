use std::{pin::Pin, str::FromStr};

use async_stream::try_stream;
use futures::Stream;
use krata::{
    idm::protocol::{
        idm_request::Request as IdmRequestType, idm_response::Response as IdmResponseType,
        IdmMetricsRequest,
    },
    v1::{
        common::{Guest, GuestState, GuestStatus},
        control::{
            control_service_server::ControlService, ConsoleDataReply, ConsoleDataRequest,
            CreateGuestReply, CreateGuestRequest, DestroyGuestReply, DestroyGuestRequest,
            ListGuestsReply, ListGuestsRequest, ReadGuestMetricsReply, ReadGuestMetricsRequest,
            ResolveGuestReply, ResolveGuestRequest, SnoopIdmReply, SnoopIdmRequest,
            WatchEventsReply, WatchEventsRequest,
        },
    },
};
use tokio::{
    select,
    sync::mpsc::{channel, Sender},
};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::{
    console::DaemonConsoleHandle, db::GuestStore, event::DaemonEventContext, idm::DaemonIdmHandle,
    metrics::idm_metric_to_api,
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
pub struct RuntimeControlService {
    events: DaemonEventContext,
    console: DaemonConsoleHandle,
    idm: DaemonIdmHandle,
    guests: GuestStore,
    guest_reconciler_notify: Sender<Uuid>,
}

impl RuntimeControlService {
    pub fn new(
        events: DaemonEventContext,
        console: DaemonConsoleHandle,
        idm: DaemonIdmHandle,
        guests: GuestStore,
        guest_reconciler_notify: Sender<Uuid>,
    ) -> Self {
        Self {
            events,
            console,
            idm,
            guests,
            guest_reconciler_notify,
        }
    }
}

enum ConsoleDataSelect {
    Read(Option<Vec<u8>>),
    Write(Option<Result<ConsoleDataRequest, tonic::Status>>),
}

#[tonic::async_trait]
impl ControlService for RuntimeControlService {
    type ConsoleDataStream =
        Pin<Box<dyn Stream<Item = Result<ConsoleDataReply, Status>> + Send + 'static>>;

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
            .send(IdmRequestType::Metrics(IdmMetricsRequest {}))
            .await
            .map_err(|error| ApiError {
                message: error.to_string(),
            })?;

        let mut reply = ReadGuestMetricsReply::default();
        if let IdmResponseType::Metrics(metrics) = response {
            reply.root = metrics.root.map(idm_metric_to_api);
        }
        Ok(Response::new(reply))
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
