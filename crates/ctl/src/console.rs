use anyhow::{anyhow, Result};
use async_stream::stream;
use crossterm::{
    terminal::{disable_raw_mode, enable_raw_mode, is_raw_mode_enabled},
    tty::IsTty,
};
use krata::{
    events::EventStream,
    v1::{
        common::ZoneStatus,
        control::{
            watch_events_reply::Event, ExecZoneReply, ExecZoneRequest, ZoneConsoleReply,
            ZoneConsoleRequest,
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
    pub async fn stdin_stream(zone: String) -> impl Stream<Item = ZoneConsoleRequest> {
        let mut stdin = stdin();
        stream! {
            yield ZoneConsoleRequest { zone_id: zone, data: vec![] };

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
                yield ZoneConsoleRequest { zone_id: String::default(), data };
            }
        }
    }

    pub async fn stdin_stream_exec(
        initial: ExecZoneRequest,
    ) -> impl Stream<Item = ExecZoneRequest> {
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
                yield ExecZoneRequest { zone_id: String::default(), task: None, data };
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

    pub async fn exec_output(mut stream: Streaming<ExecZoneReply>) -> Result<i32> {
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

                let Some(state) = zone.state else {
                    continue;
                };

                if zone.id != id {
                    continue;
                }

                if let Some(exit_info) = state.exit_info {
                    return Some(exit_info.code);
                }

                let status = state.status();
                if status == ZoneStatus::Destroying || status == ZoneStatus::Destroyed {
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
