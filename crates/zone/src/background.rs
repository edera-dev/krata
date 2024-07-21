use crate::{
    childwait::{ChildEvent, ChildWait},
    death,
    exec::ZoneExecTask,
    metrics::MetricsCollector,
};
use anyhow::Result;
use cgroups_rs::Cgroup;
use krata::idm::{
    client::{IdmClientStreamResponseHandle, IdmInternalClient},
    internal::{
        event::Event as EventType, request::Request as RequestType,
        response::Response as ResponseType, Event, ExecStreamResponseUpdate, ExitEvent,
        MetricsResponse, PingResponse, Request, Response,
    },
};
use log::debug;
use nix::unistd::Pid;
use tokio::sync::broadcast::Receiver;
use tokio::{select, sync::broadcast};

pub struct ZoneBackground {
    idm: IdmInternalClient,
    child: Pid,
    _cgroup: Cgroup,
    wait: ChildWait,
    child_receiver: Receiver<ChildEvent>,
}

impl ZoneBackground {
    pub async fn new(idm: IdmInternalClient, cgroup: Cgroup, child: Pid) -> Result<ZoneBackground> {
        let (wait, child_receiver) = ChildWait::new()?;
        Ok(ZoneBackground {
            idm,
            child,
            _cgroup: cgroup,
            wait,
            child_receiver,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut event_subscription = self.idm.subscribe().await?;
        let mut requests_subscription = self.idm.requests().await?;
        let mut request_streams_subscription = self.idm.request_streams().await?;
        loop {
            select! {
                x = event_subscription.recv() => match x {
                    Ok(_event) => {
                    },

                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("idm packet channel closed");
                        break;
                    },

                    _ => {
                        continue;
                    }
                },

                x = requests_subscription.recv() => match x {
                    Ok((id, request)) => {
                        self.handle_idm_request(id, request).await?;
                    },

                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("idm packet channel closed");
                        break;
                    },

                    _ => {
                        continue;
                    }
                },

                x = request_streams_subscription.recv() => match x {
                    Ok(handle) => {
                        self.handle_idm_stream_request(handle).await?;
                    },

                    Err(broadcast::error::RecvError::Closed) => {
                        debug!("idm packet channel closed");
                        break;
                    },

                    _ => {
                        continue;
                    }
                },

                event = self.child_receiver.recv() => match event {
                    Ok(event) => self.child_event(event).await?,
                    Err(_) => {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    async fn handle_idm_request(&mut self, id: u64, packet: Request) -> Result<()> {
        match packet.request {
            Some(RequestType::Ping(_)) => {
                self.idm
                    .respond(
                        id,
                        Response {
                            response: Some(ResponseType::Ping(PingResponse {})),
                        },
                    )
                    .await?;
            }

            Some(RequestType::Metrics(_)) => {
                let metrics = MetricsCollector::new()?;
                let root = metrics.collect()?;
                let response = Response {
                    response: Some(ResponseType::Metrics(MetricsResponse { root: Some(root) })),
                };

                self.idm.respond(id, response).await?;
            }

            _ => {}
        }
        Ok(())
    }

    async fn handle_idm_stream_request(
        &mut self,
        handle: IdmClientStreamResponseHandle<Request>,
    ) -> Result<()> {
        let wait = self.wait.clone();
        if let Some(RequestType::ExecStream(_)) = &handle.initial.request {
            tokio::task::spawn(async move {
                let exec = ZoneExecTask { wait, handle };
                if let Err(error) = exec.run().await {
                    let _ = exec
                        .handle
                        .respond(Response {
                            response: Some(ResponseType::ExecStream(ExecStreamResponseUpdate {
                                exited: true,
                                error: error.to_string(),
                                exit_code: -1,
                                stdout: vec![],
                                stderr: vec![],
                            })),
                        })
                        .await;
                }
            });
        }
        Ok(())
    }

    async fn child_event(&mut self, event: ChildEvent) -> Result<()> {
        if event.pid == self.child {
            self.idm
                .emit(Event {
                    event: Some(EventType::Exit(ExitEvent { code: event.status })),
                })
                .await?;
            death(event.status).await?;
        }
        Ok(())
    }
}
