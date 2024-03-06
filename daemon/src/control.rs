use std::{io, pin::Pin};

use async_stream::try_stream;
use futures::Stream;
use krata::control::{
    control_service_server::ControlService, ConsoleDataReply, ConsoleDataRequest,
    DestroyGuestReply, DestroyGuestRequest, GuestInfo, LaunchGuestReply, LaunchGuestRequest,
    ListGuestsReply, ListGuestsRequest,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
};
use tokio_stream::StreamExt;
use tonic::{Request, Response, Status, Streaming};

use crate::runtime::{launch::GuestLaunchRequest, Runtime};

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
    runtime: Runtime,
}

impl RuntimeControlService {
    pub fn new(runtime: Runtime) -> Self {
        Self { runtime }
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

    async fn launch_guest(
        &self,
        request: Request<LaunchGuestRequest>,
    ) -> Result<Response<LaunchGuestReply>, Status> {
        let request = request.into_inner();
        let guest: GuestInfo = self
            .runtime
            .launch(GuestLaunchRequest {
                image: &request.image,
                vcpus: request.vcpus,
                mem: request.mem,
                env: empty_vec_optional(request.env),
                run: empty_vec_optional(request.run),
                debug: false,
            })
            .await
            .map_err(ApiError::from)?
            .into();
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
            .map(GuestInfo::from)
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
            .console(&request.guest)
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
}

impl From<crate::runtime::GuestInfo> for GuestInfo {
    fn from(value: crate::runtime::GuestInfo) -> Self {
        GuestInfo {
            id: value.uuid.to_string(),
            image: value.image,
            ipv4: value.ipv4.map(|x| x.ip().to_string()).unwrap_or_default(),
            ipv6: value.ipv6.map(|x| x.ip().to_string()).unwrap_or_default(),
        }
    }
}

fn empty_vec_optional<T>(value: Vec<T>) -> Option<Vec<T>> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
