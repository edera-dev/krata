use crate::{
    childwait::{ChildEvent, ChildWait},
    death,
    metrics::MetricsCollector,
};
use anyhow::Result;
use cgroups_rs::Cgroup;
use krata::idm::{
    client::IdmInternalClient,
    internal::{
        event::Event as EventType, request::Request as RequestType,
        response::Response as ResponseType, Event, ExitEvent, MetricsResponse, PingResponse,
        Request, Response,
    },
};
use log::debug;
use nix::unistd::Pid;
use tokio::{select, sync::broadcast};

pub struct GuestBackground {
    idm: IdmInternalClient,
    child: Pid,
    _cgroup: Cgroup,
    wait: ChildWait,
}

impl GuestBackground {
    pub async fn new(
        idm: IdmInternalClient,
        cgroup: Cgroup,
        child: Pid,
    ) -> Result<GuestBackground> {
        Ok(GuestBackground {
            idm,
            child,
            _cgroup: cgroup,
            wait: ChildWait::new()?,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        let mut event_subscription = self.idm.subscribe().await?;
        let mut requests_subscription = self.idm.requests().await?;
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

                event = self.wait.recv() => match event {
                    Some(event) => self.child_event(event).await?,
                    None => {
                        break;
                    }
                }
            };
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

            None => {}
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
