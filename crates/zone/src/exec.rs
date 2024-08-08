use std::{
    collections::HashMap,
    ffi::CString,
    path::PathBuf,
};

use anyhow::{anyhow, Context, Result};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    join,
};

use krata::idm::{
    client::IdmClientStreamResponseHandle,
    internal::{
        exec_stream_request_update::Update, request::Request as RequestType,
        ExecStreamResponseUpdate,
    },
    internal::{response::Response as ResponseType, Request, Response},
};

use crate::{
    childwait::ChildWait,
    spawn::child::ChildSpec,
};

pub struct ZoneExecTask {
    pub wait: ChildWait,
    pub handle: IdmClientStreamResponseHandle<Request>,
}

impl ZoneExecTask {
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

        let cmd = start.command.clone();
        if cmd.is_empty() {
            return Err(anyhow!("command line was empty"));
        }

        let exe: PathBuf = cmd[0].clone().into();
        let cmd = cmd.into_iter().map(CString::new).collect::<Result<Vec<CString>, _>>()?;

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

        let working_dir = if start.working_directory.is_empty() {
            "/".to_string()
        } else {
            start.working_directory.clone()
        };

        let wait_rx = self.wait.subscribe().await?;

        let spec = ChildSpec {
            exe: PathBuf::from(exe),
            cmd, 
            env,
            tty: false,
            cgroup: None,
            working_dir,
            with_new_session: false,
        };

        let mut child = spec.spawn(wait_rx).context("failed to spawn")?;

        let mut stdin = child.stdin.take().context("stdin was missing")?;
        let mut stdout = child.stdout.take().context("stdout was missing")?;
        let mut stderr = child.stderr.take().context("stderr was missing")?;

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

                if stdin.write_all(&update.data).await.is_err() {
                    break;
                }
            }
        });

        let data_task = tokio::task::spawn(async move {
            let _ = join!(stdout_task, stderr_task);
            stdin_task.abort();
        });

        let exit_code = child.wait().await?;
        data_task.await?;
        let response = Response {
            response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
                exited: true,
                exit_code,
                error: String::new(),
                stdout: vec![],
                stderr: vec![],
            })),
        };
        self.handle.respond(response).await?;

        Ok(())
    }
}
