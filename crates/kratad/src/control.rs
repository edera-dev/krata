use std::{io, pin::Pin, str::FromStr};

use async_stream::try_stream;
use futures::Stream;
use krata::{
    common::{Guest, GuestState, GuestStatus},
    control::{
        control_service_server::ControlService, ConsoleDataReply, ConsoleDataRequest,
        CreateGuestReply, CreateGuestRequest, DestroyGuestReply, DestroyGuestRequest,
        ListGuestsReply, ListGuestsRequest, ResolveGuestReply, ResolveGuestRequest,
        WatchEventsReply, WatchEventsRequest,
    },
};
use kratart::Runtime;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc::Sender,
};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};
use uuid::Uuid;

use crate::{
    db::{proto::GuestEntry, GuestStore},
    event::DaemonEventContext,
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
    runtime: Runtime,
    guests: GuestStore,
    guest_reconciler_notify: Sender<Uuid>,
}

impl RuntimeControlService {
    pub fn new(
        events: DaemonEventContext,
        runtime: Runtime,
        guests: GuestStore,
        guest_reconciler_notify: Sender<Uuid>,
    ) -> Self {
        Self {
            events,
            runtime,
            guests,
            guest_reconciler_notify,
        }
    }
}

enum ConsoleDataSelect {
    Read(io::Result<usize>),
    Write(Option<Result<ConsoleDataRequest, tonic::Status>>),
}

#[tonic::async_trait]
impl ControlService for RuntimeControlService {
    type ConsoleDataStream =
        Pin<Box<dyn Stream<Item = Result<ConsoleDataReply, Status>> + Send + 'static>>;

    type WatchEventsStream =
        Pin<Box<dyn Stream<Item = Result<WatchEventsReply, Status>> + Send + 'static>>;

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
                GuestEntry {
                    id: uuid.to_string(),
                    guest: Some(Guest {
                        id: uuid.to_string(),
                        state: Some(GuestState {
                            status: GuestStatus::Starting.into(),
                            network: None,
                            exit_info: None,
                            error_info: None,
                        }),
                        spec: Some(spec),
                    }),
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
        let Some(mut entry) = self.guests.read(uuid).await.map_err(ApiError::from)? else {
            return Err(ApiError {
                message: "guest not found".to_string(),
            }
            .into());
        };
        let Some(ref mut guest) = entry.guest else {
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
            .update(uuid, entry)
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
        let guests = guests
            .into_values()
            .filter_map(|entry| entry.guest)
            .collect::<Vec<Guest>>();
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
            .filter_map(|entry| entry.guest)
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
        let mut console = self.runtime.console(uuid).await.map_err(ApiError::from)?;

        let output = try_stream! {
            let mut buffer: Vec<u8> = vec![0u8; 256];
            loop {
                let what = select! {
                    x = console.read_handle.read(&mut buffer) => ConsoleDataSelect::Read(x),
                    x = input.next() => ConsoleDataSelect::Write(x),
                };

                match what {
                    ConsoleDataSelect::Read(result) => {
                        let size = result?;
                        let data = buffer[0..size].to_vec();
                        yield ConsoleDataReply { data, };
                    },

                    ConsoleDataSelect::Write(Some(request)) => {
                        let request = request?;
                        if !request.data.is_empty() {
                            console.write_handle.write_all(&request.data).await?;
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
}
