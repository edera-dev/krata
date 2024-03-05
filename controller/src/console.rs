use std::{
    io::{stdin, stdout},
    os::fd::{AsRawFd, FromRawFd},
};

use anyhow::Result;
use krata::{
    control::{ConsoleStreamUpdate, StreamUpdate},
    stream::StreamContext,
};
use log::debug;
use std::process::exit;
use termion::raw::IntoRawMode;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
    select,
};

pub struct XenConsole {
    stream: StreamContext,
}

impl XenConsole {
    pub async fn new(stream: StreamContext) -> Result<XenConsole> {
        Ok(XenConsole { stream })
    }

    pub async fn attach(self) -> Result<()> {
        let stdin = unsafe { File::from_raw_fd(stdin().as_raw_fd()) };
        let terminal = stdout().into_raw_mode()?;
        let stdout = unsafe { File::from_raw_fd(terminal.as_raw_fd()) };

        if let Err(error) = XenConsole::process(stdin, stdout, self.stream).await {
            debug!("failed to process console stream: {}", error);
        }

        Ok(())
    }

    async fn process(mut stdin: File, mut stdout: File, mut stream: StreamContext) -> Result<()> {
        let mut buffer = vec![0u8; 60];
        loop {
            select! {
                x = stream.receiver.recv() => match x {
                    Some(StreamUpdate::ConsoleStream(update)) => {
                        stdout.write_all(&update.data).await?;
                        stdout.flush().await?;
                    },

                    None => {
                        break;
                    }
                },

                x = stdin.read(&mut buffer) => match x {
                    Ok(size) => {
                        if size == 1 && buffer[0] == 0x1d {
                            exit(0);
                        }

                        let data = buffer[0..size].to_vec();
                        stream.send(StreamUpdate::ConsoleStream(ConsoleStreamUpdate {
                            data,
                        })).await?;
                    },

                    Err(error) => {
                        return Err(error.into());
                    }
                }
            };
        }
        Ok(())
    }
}
