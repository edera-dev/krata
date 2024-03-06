use std::{
    io::stdout,
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::Result;
use async_stream::stream;
use krata::control::{ConsoleDataReply, ConsoleDataRequest};
use log::debug;
use termion::raw::IntoRawMode;
use tokio::{
    fs::File,
    io::{stdin, AsyncReadExt, AsyncWriteExt},
};
use tokio_stream::{Stream, StreamExt};
use tonic::Streaming;

pub struct StdioConsoleStream;

impl StdioConsoleStream {
    pub async fn stdin_stream(guest: String) -> impl Stream<Item = ConsoleDataRequest> {
        let mut stdin = stdin();
        stream! {
            yield ConsoleDataRequest { guest, data: vec![] };

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
                yield ConsoleDataRequest { guest: String::default(), data };
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
}
