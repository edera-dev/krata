use std::{
    io::stdout,
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::Result;
use async_stream::stream;
use krata::control::{
    watch_events_reply::Event, ConsoleDataReply, ConsoleDataRequest, WatchEventsReply,
};
use log::{debug, error, warn};
use termion::raw::IntoRawMode;
use tokio::{
    fs::File,
    io::{stdin, AsyncReadExt, AsyncWriteExt},
    task::JoinHandle,
};
use tokio_stream::{Stream, StreamExt};
use tonic::Streaming;

pub struct StdioConsoleStream;

impl StdioConsoleStream {
    pub async fn stdin_stream(guest: String) -> impl Stream<Item = ConsoleDataRequest> {
        let mut stdin = stdin();
        stream! {
            yield ConsoleDataRequest { guest_id: guest, data: vec![] };

            let mut buffer = vec![0u8; 60];
            loop {
                let size = match stdin.read(&mut buffer).await {
                    Ok(size) => size,
                    Err(error) => {
                        debug!("failed to read stdin: {}", error);
                        break;
                    }
                };
                let data = buffer[0..size].to_vec();
                if size == 1 && buffer[0] == 0x1d {
                    break;
                }
                yield ConsoleDataRequest { guest_id: String::default(), data };
            }
        }
    }

    pub async fn stdout(mut stream: Streaming<ConsoleDataReply>) -> Result<()> {
        let terminal = stdout().into_raw_mode()?;
        let mut stdout = unsafe { File::from_raw_fd(terminal.as_raw_fd()) };
        while let Some(reply) = stream.next().await {
            let reply = reply?;
            if reply.data.is_empty() {
                continue;
            }
            stdout.write_all(&reply.data).await?;
            stdout.flush().await?;
        }
        Ok(())
    }

    pub async fn guest_exit_hook(
        id: String,
        mut events: Streaming<WatchEventsReply>,
    ) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            while let Some(result) = events.next().await {
                match result {
                    Err(error) => {
                        error!("failed to handle events for exit hook: {}", error);
                        break;
                    }

                    Ok(reply) => {
                        let Some(event) = reply.event else {
                            continue;
                        };

                        match event {
                            Event::GuestExited(exit) => {
                                if exit.guest_id == id {
                                    std::process::exit(exit.code);
                                }
                            }

                            Event::GuestDestroyed(destroy) => {
                                if destroy.guest_id == id {
                                    warn!("attached guest destroyed");
                                    std::process::exit(1);
                                }
                            }

                            _ => {
                                continue;
                            }
                        }
                    }
                }
            }
        }))
    }
}
