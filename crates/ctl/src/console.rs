use anyhow::{anyhow, Result};
use async_stream::stream;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled},
    tty::IsTty,
};
use krata::v1::common::ZoneState;
use krata::{
    events::EventStream,
    v1::control::{
        watch_events_reply::Event, ExecInsideZoneReply, ExecInsideZoneRequest, ZoneConsoleReply,
        ZoneConsoleRequest,
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
    pub async fn stdin_stream(
        zone: String,
        replay_history: bool,
    ) -> impl Stream<Item = ZoneConsoleRequest> {
        let mut stdin = stdin();
        stream! {
            yield ZoneConsoleRequest { zone_id: zone, replay_history, data: vec![] };

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
                yield ZoneConsoleRequest { zone_id: String::default(), replay_history, data };
            }
        }
    }

    pub async fn stdin_stream_exec(
        initial: ExecInsideZoneRequest,
    ) -> impl Stream<Item = ExecInsideZoneRequest> {
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
                let stdin = buffer[0..size].to_vec();
                if size == 1 && buffer[0] == 0x1d {
                    break;
                }
                let stdin_closed = size == 0;
                yield ExecInsideZoneRequest { zone_id: String::default(), task: None, stdin, stdin_closed, };
                if stdin_closed {
                    break;
                }
            }
        }
    }

    pub async fn stdout(mut stream: Streaming<ZoneConsoleReply>, raw: bool) -> Result<()> {
        if raw && stdin().is_tty() {
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

    pub async fn exec_output(mut stream: Streaming<ExecInsideZoneReply>, raw: bool) -> Result<i32> {
        if raw && stdin().is_tty() {
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
                return if reply.error.is_empty() {
                    Ok(reply.exit_code)
                } else {
                    Err(anyhow!("exec failed: {}", reply.error))
                };
            }
        }
        Ok(-1)
    }

    pub async fn zone_exit_hook(
        id: String,
        events: EventStream,
    ) -> Result<JoinHandle<Option<i32>>> {
        Ok(tokio::task::spawn(async move {
            let mut stream = events.subscribe();
            while let Ok(event) = stream.recv().await {
                let Event::ZoneChanged(changed) = event;
                let Some(zone) = changed.zone else {
                    continue;
                };

                let Some(status) = zone.status else {
                    continue;
                };

                if zone.id != id {
                    continue;
                }

                if let Some(exit_status) = status.exit_status {
                    return Some(exit_status.code);
                }

                let state = status.state();
                if state == ZoneState::Destroying || state == ZoneState::Destroyed {
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
