use std::pin::Pin;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use async_stream::try_stream;
use tokio::select;
use tokio_stream::{Stream, StreamExt};
use tonic::{Status, Streaming};
use uuid::Uuid;

use krata::idm::internal::Request;
use krata::{
    idm::internal::{
        exec_stream_request_update::Update, request::Request as IdmRequestType,
        response::Response as IdmResponseType, ExecEnvVar, ExecStreamRequestStart,
        ExecStreamRequestStdin, ExecStreamRequestUpdate, Request as IdmRequest,
    },
    v1::control::{ExecInsideZoneReply, ExecInsideZoneRequest},
};

use crate::control::ApiError;
use crate::idm::DaemonIdmHandle;

pub struct ExecInsideZoneRpc {
    idm: DaemonIdmHandle,
}

impl ExecInsideZoneRpc {
    pub fn new(idm: DaemonIdmHandle) -> Self {
        Self { idm }
    }

    pub async fn process(
        self,
        mut input: Streaming<ExecInsideZoneRequest>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ExecInsideZoneReply, Status>> + Send + 'static>>>
    {
        let Some(request) = input.next().await else {
            return Err(anyhow!("expected to have at least one request"));
        };
        let request = request?;

        let Some(task) = request.task else {
            return Err(anyhow!("task is missing"));
        };

        let uuid = Uuid::from_str(&request.zone_id)?;
        let idm = self.idm.client(uuid).await?;

        let idm_request = Request {
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
                    tty: task.tty,
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
                            if !update.stdin.is_empty() {
                                let _ = handle.update(IdmRequest {
                                    request: Some(IdmRequestType::ExecStream(ExecStreamRequestUpdate {
                                        update: Some(Update::Stdin(ExecStreamRequestStdin {
                                            data: update.stdin,
                                            closed: update.stdin_closed,
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
                                stderr: update.stderr,
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

        Ok(Box::pin(output))
    }
}
