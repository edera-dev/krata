use anyhow::{anyhow, Result};
use async_stream::stream;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled},
    tty::IsTty,
};
use krata::{
    events::EventStream,
    v1::{
        common::GuestStatus,
        control::{
            watch_events_reply::Event, ConsoleDataReply, ConsoleDataRequest, ExecGuestReply,
            ExecGuestRequest,
        },
    },
};
use log::debug;
use tokio::{
    io::{stderr, stdin, stdout, AsyncReadExt, AsyncWriteExt},
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

    pub async fn stdin_stream_exec(
        initial: ExecGuestRequest,
    ) -> impl Stream<Item = ExecGuestRequest> {
        let mut stdin = stdin();
        stream! {
            yield initial;

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

                let closed = size == 0;

                yield ExecGuestRequest { guest_id: String::default(), task: None, tty: false, stdin: data, stdin_closed: closed };
                if closed {
                    break;
                }
            }
        }
    }

    pub async fn stdout(mut stream: Streaming<ConsoleDataReply>) -> Result<()> {
        if stdin().is_tty() {
            enable_raw_mode()?;
            StdioConsoleStream::register_terminal_restore_hook()?;
        }
        let mut stdout = stdout();
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

    pub async fn exec_output(tty: bool, mut stream: Streaming<ExecGuestReply>) -> Result<i32> {
        if tty && stdin().is_tty() {
            enable_raw_mode()?;
            StdioConsoleStream::register_terminal_restore_hook()?;
        }
        let mut stdout = stdout();
        let mut stderr = stderr();
        while let Some(reply) = stream.next().await {
            let reply = reply?;
            if !reply.stdout.is_empty() {
                stdout.write_all(&reply.stdout).await?;
                stdout.flush().await?;
            }

            if !reply.stderr.is_empty() {
                stderr.write_all(&reply.stderr).await?;
                stderr.flush().await?;
            }

            if reply.exited {
                if reply.error.is_empty() {
                    return Ok(reply.exit_code);
                } else {
                    return Err(anyhow!("exec failed: {}", reply.error));
                }
            }
        }
        Ok(-1)
    }

    pub async fn guest_exit_hook(
        id: String,
        events: EventStream,
    ) -> Result<JoinHandle<Option<i32>>> {
        Ok(tokio::task::spawn(async move {
            let mut stream = events.subscribe();
            while let Ok(event) = stream.recv().await {
                let Event::GuestChanged(changed) = event;
                let Some(guest) = changed.guest else {
                    continue;
                };

                let Some(state) = guest.state else {
                    continue;
                };

                if guest.id != id {
                    continue;
                }

                if let Some(exit_info) = state.exit_info {
                    return Some(exit_info.code);
                }

                let status = state.status();
                if status == GuestStatus::Destroying || status == GuestStatus::Destroyed {
                    return Some(10);
                }
            }
            None
        }))
    }

    fn register_terminal_restore_hook() -> Result<()> {
        if stdin().is_tty() {
            ctrlc::set_handler(move || {
                StdioConsoleStream::restore_terminal_mode();
            })?;
        }
        Ok(())
    }

    pub fn restore_terminal_mode() {
        if is_raw_mode_enabled().unwrap_or(false) {
            let _ = disable_raw_mode();
        }
    }
}
