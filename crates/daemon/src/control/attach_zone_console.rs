use std::pin::Pin;
use std::str::FromStr;

use anyhow::{anyhow, Result};
use async_stream::try_stream;
use tokio::select;
use tokio::sync::mpsc::channel;
use tokio_stream::{Stream, StreamExt};
use tonic::{Status, Streaming};
use uuid::Uuid;

use krata::v1::control::{ZoneConsoleReply, ZoneConsoleRequest};

use crate::console::DaemonConsoleHandle;
use crate::control::ApiError;

enum ConsoleDataSelect {
    Read(Option<Vec<u8>>),
    Write(Option<Result<ZoneConsoleRequest, Status>>),
}

pub struct AttachZoneConsoleRpc {
    console: DaemonConsoleHandle,
}

impl AttachZoneConsoleRpc {
    pub fn new(console: DaemonConsoleHandle) -> Self {
        Self { console }
    }

    pub async fn process(
        self,
        mut input: Streaming<ZoneConsoleRequest>,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<ZoneConsoleReply, Status>> + Send + 'static>>>
    {
        let Some(request) = input.next().await else {
            return Err(anyhow!("expected to have at least one request"));
        };
        let request = request?;
        let uuid = Uuid::from_str(&request.zone_id)?;
        let (sender, mut receiver) = channel(100);
        let console = self
            .console
            .attach(uuid, sender)
            .await
            .map_err(|error| anyhow!("failed to attach to console: {}", error))?;

        let output = try_stream! {
            if request.replay_history {
                yield ZoneConsoleReply { data: console.initial.clone(), };
            }
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
        Ok(Box::pin(output))
    }
}
