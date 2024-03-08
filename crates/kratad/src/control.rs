use std::{io, pin::Pin};

use async_stream::try_stream;
use futures::Stream;
use krata::control::{
    control_service_server::ControlService, guest_image_spec::Image, ConsoleDataReply,
    ConsoleDataRequest, DestroyGuestReply, DestroyGuestRequest, GuestImageSpec, GuestInfo,
    GuestNetworkInfo, GuestOciImageSpec, LaunchGuestReply, LaunchGuestRequest, ListGuestsReply,
    ListGuestsRequest, WatchEventsReply, WatchEventsRequest,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

use crate::event::DaemonEventContext;
use kratart::{launch::GuestLaunchRequest, Runtime};

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
}

impl RuntimeControlService {
    pub fn new(events: DaemonEventContext, runtime: Runtime) -> Self {
        Self { events, runtime }
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

    async fn launch_guest(
        &self,
        request: Request<LaunchGuestRequest>,
    ) -> Result<Response<LaunchGuestReply>, Status> {
        let request = request.into_inner();
        let Some(image) = request.image else {
            return Err(ApiError {
                message: "image spec not provider".to_string(),
            }
            .into());
        };
        let oci = match image.image {
            Some(Image::Oci(oci)) => oci,
            None => {
                return Err(ApiError {
                    message: "image spec not provided".to_string(),
                }
                .into())
            }
        };
        let guest: GuestInfo = convert_guest_info(
            self.runtime
                .launch(GuestLaunchRequest {
                    image: &oci.image,
                    vcpus: request.vcpus,
                    mem: request.mem,
                    env: empty_vec_optional(request.env),
                    run: empty_vec_optional(request.run),
                    debug: false,
                })
                .await
                .map_err(ApiError::from)?,
        );
        Ok(Response::new(LaunchGuestReply { guest: Some(guest) }))
    }

    async fn destroy_guest(
        &self,
        request: Request<DestroyGuestRequest>,
    ) -> Result<Response<DestroyGuestReply>, Status> {
        let request = request.into_inner();
        self.runtime
            .destroy(&request.guest_id)
            .await
            .map_err(ApiError::from)?;
        Ok(Response::new(DestroyGuestReply {}))
    }

    async fn list_guests(
        &self,
        request: Request<ListGuestsRequest>,
    ) -> Result<Response<ListGuestsReply>, Status> {
        let _ = request.into_inner();
        let guests = self.runtime.list().await.map_err(ApiError::from)?;
        let guests = guests
            .into_iter()
            .map(convert_guest_info)
            .collect::<Vec<GuestInfo>>();
        Ok(Response::new(ListGuestsReply { guests }))
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
        let mut console = self
            .runtime
            .console(&request.guest_id)
            .await
            .map_err(ApiError::from)?;

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

fn empty_vec_optional<T>(value: Vec<T>) -> Option<Vec<T>> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}

fn convert_guest_info(value: kratart::GuestInfo) -> GuestInfo {
    GuestInfo {
        id: value.uuid.to_string(),
        image: Some(GuestImageSpec {
            image: Some(Image::Oci(GuestOciImageSpec { image: value.image })),
        }),
        network: Some(GuestNetworkInfo {
            ipv4: value.ipv4.map(|x| x.ip().to_string()).unwrap_or_default(),
            ipv6: value.ipv6.map(|x| x.ip().to_string()).unwrap_or_default(),
        }),
    }
}
