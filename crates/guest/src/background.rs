use crate::{
    childwait::{ChildEvent, ChildWait},
    death,
    metrics::MetricsCollector,
};
use anyhow::Result;
use cgroups_rs::Cgroup;
use krata::idm::{
    client::IdmClient,
    protocol::{
        idm_event::Event, idm_request::Request, idm_response::Response, IdmEvent, IdmExitEvent,
        IdmMetricsResponse, IdmPingResponse, IdmRequest,
    },
};
use log::debug;
use nix::unistd::Pid;
use tokio::{select, sync::broadcast};

pub struct GuestBackground {
    idm: IdmClient,
    child: Pid,
    _cgroup: Cgroup,
    wait: ChildWait,
}

impl GuestBackground {
    pub async fn new(idm: IdmClient, cgroup: Cgroup, child: Pid) -> Result<GuestBackground> {
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
                    Ok(request) => {
                        self.handle_idm_request(request).await?;
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

    async fn handle_idm_request(&mut self, packet: IdmRequest) -> Result<()> {
        let id = packet.id;

        match packet.request {
            Some(Request::Ping(_)) => {
                self.idm
                    .respond(id, Response::Ping(IdmPingResponse {}))
                    .await?;
            }

            Some(Request::Metrics(_)) => {
                let metrics = MetricsCollector::new()?;
                let root = metrics.collect()?;
                let response = IdmMetricsResponse { root: Some(root) };

                self.idm.respond(id, Response::Metrics(response)).await?;
            }

            None => {}
        }
        Ok(())
    }

    async fn child_event(&mut self, event: ChildEvent) -> Result<()> {
        if event.pid == self.child {
            self.idm
                .emit(IdmEvent {
                    event: Some(Event::Exit(IdmExitEvent { code: event.status })),
                })
                .await?;
            death(event.status).await?;
        }
        Ok(())
    }
}
