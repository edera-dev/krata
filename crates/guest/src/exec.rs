use std::{
  collections::HashMap,
  ffi::CString,
  io::{Read, Write},
  sync::Arc,
  time::Duration,
};

use anyhow::{anyhow, bail, Result};
use log::debug;

use futures::future::{BoxFuture, FutureExt};
use pty_process::{blocking::Pty, Size};
use tokio::{
  io::{AsyncReadExt, AsyncWriteExt},
  runtime::Handle,
  sync::{broadcast::Receiver, Mutex},
  task,
  time::sleep,
  join, try_join,
};

use krata::idm::{
  client::IdmClientStreamResponseHandle,
  internal::{
    exec_stream_request_update::Update, request::Request as RequestType,
    ExecStreamResponseUpdate,
  },
  internal::{response::Response as ResponseType, Request, Response},
};

use crate::{childwait::ChildEvent, spawn::child::{Child, ChildSpec}};

pub struct GuestExecTask {
  pub handle: IdmClientStreamResponseHandle<Request>,
  pub reaper_rx: Receiver<ChildEvent>,
}

impl GuestExecTask {
  pub async fn run(&mut self) -> Result<()> {
    let receiver = self.handle.take().await?;

    let Some(ref request) = self.handle.initial.request else {
      return Err(anyhow!("request was empty"));
    };

    let RequestType::ExecStream(update) = request else {
      return Err(anyhow!("request was not an exec update"));
    };

    let Some(Update::Start(ref start)) = update.update else {
      return Err(anyhow!("first request did not contain a start update"));
    };

    debug!("exec task started");

    // The command, exe + args
    let cmd = start.command.clone();
    if cmd.is_empty() {
      return Err(anyhow!("command line was empty"));
    }

    // Clone the exe path from the args
    let exe = cmd[0].clone();

    // With the exe popped, we can convert the rest to CStrings, the exe we want to save as a Path.
    let args = cmd.into_iter()
      .map(CString::new)
      .collect::<Result<Vec<CString>, _>>()?;

    let mut env = HashMap::new();
    for entry in &start.environment {
      env.insert(entry.key.clone(), entry.value.clone());
    }

    if !env.contains_key("PATH") {
      debug!("missing PATH, supplying default");
      env.insert(
        "PATH".to_string(),
        "/bin:/usr/bin:/usr/local/bin".to_string(),
      );
    }

    for (k, v) in &env {
      debug!("Env var {k}={v}");
    }

    let working_dir = if start.working_directory.is_empty() {
      "/".to_string()
    } else {
      start.working_directory.clone()
    };

    // TODO: impl From<Update::Start> for ChildSpec
    let child_spec = ChildSpec {
      cmd: exe.into(),
      args,
      env,
      working_dir,
      cgroup: None,
      with_new_session: true,
      tty: start.tty,
    //strategy: Strategy {
    //  fatal: false,
    //  retry: None,
    //  success_code: Some(0),
    //},
    };

    //let mut child = self.supervisor.spawn_process(child_spec).await;
    let mut child = match child_spec.spawn() {
      Ok(c) => c,
      Err(e) => {
        let response = Response {
          response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
            exited: true,
            exit_code: -99,
            error: e.to_string(),
            stdout: vec![],
            stderr: vec![],
          }))
        };
        let _ = self.handle.respond(response).await;
        return Ok(());
      },
    };

    let stdin  = child.stdin.take().expect("stdin was missing");
    let stdout = child.stdout.take().expect("stdout was missing");
    let stderr = child.stderr.take().expect("stderr was missing");

    let stdout_handle = self.handle.clone();
    let stdout_builder = move || {
      let mut stdout = stdout;
      let stdout_handle = stdout_handle;
      async move {
        let mut buf = vec![0u8; 8192];
        loop {
          let len = stdout.read_buf(&mut buf).await?;
          let response = Response {
            response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
              exited: false,
              exit_code: 0,
              error: String::new(),
              stdout: buf[..len].into(),
              stderr: vec![],
            })),
          };
          let _ = stdout_handle.respond(response).await;
        }
      }
    };

    let stderr_handle = self.handle.clone();
    let stderr_builder = move || { 
      let mut stderr = stderr;
      let stderr_handle = stderr_handle;
      async move {
        let mut buf = vec![0u8; 8192];
        loop {
          let len = stderr.read_buf(&mut buf).await?;
          let response = Response {
            response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
              exited: false,
              exit_code: 0,
              error: String::new(),
              stdout: vec![],
              stderr: buf[..len].into(),
            })),
          };
          let _ = stderr_handle.respond(response).await;
        }
      }
    };

    let stdin_builder = move || {
      let mut stdin = stdin;
      let receiver = Mutex::new(receiver);

      async move {
        let mut receiver = receiver.lock().await;
        loop {
          let request = match tokio::time::timeout(Duration::from_millis(2000), receiver.recv()).await {
            Ok(Some(r)) => r,
            _ => bail!("Receiver channel has died"),
          };
   
          let Some(RequestType::ExecStream(update)) = request.request else {
            continue;
          };
   
          let Some(Update::Stdin(update)) = update.update else {
            continue;
          };
   
          if !update.data.is_empty() {
            stdin.write_all(&update.data).await?;
          }
   
          if update.closed {
            bail!("Exec stream closed");
          }
        }
      }
    };

    let stdin_handle:  task::JoinHandle<Result<(), anyhow::Error>>
      = task::spawn(stdin_builder());
    let stdout_handle: task::JoinHandle<Result<(), anyhow::Error>>
      = task::spawn(stdout_builder());
    let stderr_handle: task::JoinHandle<Result<(), anyhow::Error>>
      = task::spawn(stderr_builder());

    // let stdin_handle = self.supervisor.spawn_async(TaskSpec {
    //   builder: Arc::new(stdin_builder),
    //   strategy: stdio_strategy.clone(),
    // });

    // let stdout_handle = self.supervisor.spawn_async(TaskSpec {
    //   builder: Arc::new(stdout_builder),
    //   strategy: stdio_strategy.clone(),
    // });

    // let stderr_handle = self.supervisor.spawn_async(TaskSpec {
    //   builder: Arc::new(stderr_builder),
    //   strategy: stdio_strategy.clone(),
    // });
    

    let exit_code = loop {
      if let Ok(c) = self.reaper_rx.recv().await {
        if c.pid.as_raw() == child.pid() {
          break c.status;
        }
      }
    };

    join!(stdin_handle, stdout_handle, stderr_handle);

    let response = Response {
      response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
        exited: true,
        exit_code,
        error: String::from(""), // TODO: get this out of the Supervisor
        stdout: vec![],
        stderr: vec![],
      })),
    };
    self.handle.respond(response).await?;

    
    Ok(())
  }
}

