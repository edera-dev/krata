use crate::{
    childwait::{ChildEvent, ChildWait},
    death,
};
use anyhow::Result;
use cgroups_rs::Cgroup;
use krata::idm::{
    client::IdmClient,
    protocol::{idm_event::Event, IdmEvent, IdmExitEvent},
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
