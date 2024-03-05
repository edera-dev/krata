use anyhow::{anyhow, Result};
use krata::control::{ConsoleStreamResponse, ConsoleStreamUpdate, Request, Response, StreamUpdate};
use log::warn;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    select,
};

use crate::{
    listen::DaemonRequestHandler,
    runtime::{console::XenConsole, Runtime},
};
use krata::stream::{ConnectionStreams, StreamContext};
pub struct ConsoleStreamRequestHandler {}

impl Default for ConsoleStreamRequestHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ConsoleStreamRequestHandler {
    pub fn new() -> Self {
        Self {}
    }

    async fn link_console_stream(mut stream: StreamContext, mut console: XenConsole) -> Result<()> {
        loop {
            let mut buffer = vec![0u8; 256];
            select! {
                x = console.read_handle.read(&mut buffer) => match x {
                    Ok(size) => {
                        let data = buffer[0..size].to_vec();
                        let update = StreamUpdate::ConsoleStream(ConsoleStreamUpdate {
                            data,
                        });
                        stream.send(update).await?;
                    },

                    Err(error) => {
                        return Err(error.into());
                    }
                },

                x = stream.receiver.recv() => match x {
                    Some(StreamUpdate::ConsoleStream(update)) => {
                        console.write_handle.write_all(&update.data).await?;
                    }

                    None => {
                        break;
                    }
                }
            };
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl DaemonRequestHandler for ConsoleStreamRequestHandler {
    fn accepts(&self, request: &Request) -> bool {
        matches!(request, Request::ConsoleStream(_))
    }

    async fn handle(
        &self,
        streams: ConnectionStreams,
        runtime: Runtime,
        request: Request,
    ) -> Result<Response> {
        let console_stream = match request {
            Request::ConsoleStream(stream) => stream,
            _ => return Err(anyhow!("unknown request")),
        };
        let console = runtime.console(&console_stream.guest).await?;
        let stream = streams.open().await?;
        let id = stream.id;
        tokio::task::spawn(async move {
            if let Err(error) =
                ConsoleStreamRequestHandler::link_console_stream(stream, console).await
            {
                warn!("failed to process console stream: {}", error);
            }
        });

        Ok(Response::ConsoleStream(ConsoleStreamResponse {
            stream: id,
        }))
    }
}
