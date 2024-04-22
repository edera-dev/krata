use std::{collections::HashMap, process::Stdio, time::Duration};

use anyhow::{anyhow, Result};
use krata::idm::{
    client::IdmClientStreamResponseHandle,
    internal::{
        exec_stream_request_update::Update, request::Request as RequestType,
        ExecStreamResponseUpdate,
    },
    internal::{response::Response as ResponseType, Request, Response},
};
use pty_process::{Pty, Size};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    join,
    process::{Child, Command},
    time::sleep,
};

pub struct GuestExecTask {
    pub handle: IdmClientStreamResponseHandle<Request>,
}

impl GuestExecTask {
    pub async fn run(&self) -> Result<()> {
        let mut receiver = self.handle.take().await?;

        let Some(ref request) = self.handle.initial.request else {
            return Err(anyhow!("request was empty"));
        };

        let RequestType::ExecStream(update) = request else {
            return Err(anyhow!("request was not an exec update"));
        };

        let Some(Update::Start(ref start)) = update.update else {
            return Err(anyhow!("first request did not contain a start update"));
        };

        let mut cmd = start.command.clone();
        if cmd.is_empty() {
            return Err(anyhow!("command line was empty"));
        }
        let exe = cmd.remove(0);
        let mut env = HashMap::new();
        for entry in &start.environment {
            env.insert(entry.key.clone(), entry.value.clone());
        }

        if !env.contains_key("PATH") {
            env.insert(
                "PATH".to_string(),
                "/bin:/usr/bin:/usr/local/bin".to_string(),
            );
        }

        let dir = if start.working_directory.is_empty() {
            "/".to_string()
        } else {
            start.working_directory.clone()
        };

        if start.tty {
            let pty = Pty::new().map_err(|error| anyhow!("unable to allocate pty: {}", error))?;
            pty.resize(Size::new(24, 80))?;
            let mut child = ChildDropGuard {
                inner: pty_process::Command::new(exe)
                    .args(cmd)
                    .envs(env)
                    .current_dir(dir)
                    .spawn(
                        &pty.pts()
                            .map_err(|error| anyhow!("unable to allocate pts: {}", error))?,
                    )
                    .map_err(|error| anyhow!("failed to spawn: {}", error))?,
                kill: true,
            };
            let (mut read, mut write) = pty.into_split();

            let pty_read_handle = self.handle.clone();
            let pty_read_task = tokio::task::spawn(async move {
                let mut stdout_buffer = vec![0u8; 8 * 1024];
                loop {
                    let Ok(size) = read.read(&mut stdout_buffer).await else {
                        break;
                    };
                    if size > 0 {
                        let response = Response {
                            response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
                                exited: false,
                                exit_code: 0,
                                error: String::new(),
                                stdout: stdout_buffer[0..size].to_vec(),
                                stderr: vec![],
                            })),
                        };
                        let _ = pty_read_handle.respond(response).await;
                    } else {
                        break;
                    }
                }
            });

            let stdin_task = tokio::task::spawn(async move {
                loop {
                    let Some(request) = receiver.recv().await else {
                        break;
                    };

                    let Some(RequestType::ExecStream(update)) = request.request else {
                        continue;
                    };

                    let Some(Update::Stdin(update)) = update.update else {
                        continue;
                    };

                    if !update.data.is_empty() && write.write_all(&update.data).await.is_err() {
                        break;
                    }

                    if update.closed {
                        break;
                    }
                }
            });

            let mut result = child.inner.wait().await;
            if result.is_err() {
                sleep(Duration::from_millis(10)).await;
                if let Ok(Some(status)) = child.inner.try_wait() {
                    result = Ok(status);
                }
            }
            let code = result.as_ref().ok().and_then(|x| x.code()).unwrap_or(-1);
            let error = result
                .as_ref()
                .map_err(|x| x.to_string())
                .err()
                .unwrap_or_default();

            let _ = pty_read_task.await;
            stdin_task.abort();
            let _ = stdin_task.await;

            let response = Response {
                response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
                    exited: true,
                    exit_code: code,
                    error,
                    stdout: vec![],
                    stderr: vec![],
                })),
            };
            self.handle.respond(response).await?;
            child.kill = false;
        } else {
            let mut child = ChildDropGuard {
                inner: Command::new(exe)
                    .args(cmd)
                    .envs(env)
                    .current_dir(dir)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|error| anyhow!("failed to spawn: {}", error))?,
                kill: true,
            };
            let mut stdin = child
                .inner
                .stdin
                .take()
                .ok_or_else(|| anyhow!("stdin was missing"))?;
            let mut stdout = child
                .inner
                .stdout
                .take()
                .ok_or_else(|| anyhow!("stdout was missing"))?;
            let mut stderr = child
                .inner
                .stderr
                .take()
                .ok_or_else(|| anyhow!("stderr was missing"))?;

            let stdout_handle = self.handle.clone();
            let stdout_task = tokio::task::spawn(async move {
                let mut stdout_buffer = vec![0u8; 8 * 1024];
                loop {
                    let Ok(size) = stdout.read(&mut stdout_buffer).await else {
                        break;
                    };
                    if size > 0 {
                        let response = Response {
                            response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
                                exited: false,
                                exit_code: 0,
                                error: String::new(),
                                stdout: stdout_buffer[0..size].to_vec(),
                                stderr: vec![],
                            })),
                        };
                        let _ = stdout_handle.respond(response).await;
                    } else {
                        break;
                    }
                }
            });

            let stderr_handle = self.handle.clone();
            let stderr_task = tokio::task::spawn(async move {
                let mut stderr_buffer = vec![0u8; 8 * 1024];
                loop {
                    let Ok(size) = stderr.read(&mut stderr_buffer).await else {
                        break;
                    };
                    if size > 0 {
                        let response = Response {
                            response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
                                exited: false,
                                exit_code: 0,
                                error: String::new(),
                                stdout: vec![],
                                stderr: stderr_buffer[0..size].to_vec(),
                            })),
                        };
                        let _ = stderr_handle.respond(response).await;
                    } else {
                        break;
                    }
                }
            });

            let stdin_task = tokio::task::spawn(async move {
                loop {
                    let Some(request) = receiver.recv().await else {
                        break;
                    };

                    let Some(RequestType::ExecStream(update)) = request.request else {
                        continue;
                    };

                    let Some(Update::Stdin(update)) = update.update else {
                        continue;
                    };

                    if !update.data.is_empty() && stdin.write_all(&update.data).await.is_err() {
                        break;
                    }

                    if update.closed {
                        break;
                    }
                }
            });

            let mut result = child.inner.wait().await;
            if result.is_err() {
                sleep(Duration::from_millis(10)).await;
                if let Ok(Some(status)) = child.inner.try_wait() {
                    result = Ok(status);
                }
            }
            let code = result.as_ref().ok().and_then(|x| x.code()).unwrap_or(-1);
            let error = result
                .as_ref()
                .map_err(|x| x.to_string())
                .err()
                .unwrap_or_default();

            let _ = join!(stdout_task, stderr_task);
            stdin_task.abort();

            let response = Response {
                response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
                    exited: true,
                    exit_code: code,
                    error,
                    stdout: vec![],
                    stderr: vec![],
                })),
            };
            self.handle.respond(response).await?;
            child.kill = false;
        }
        Ok(())
    }
}

struct ChildDropGuard {
    pub inner: Child,
    pub kill: bool,
}

impl Drop for ChildDropGuard {
    fn drop(&mut self) {
        if self.kill {
            drop(self.inner.start_kill());
        }
    }
}
