use crate::{
    childwait::{ChildEvent, ChildWait},
    death,
};
use anyhow::Result;
use cgroups_rs::Cgroup;
use krata::idm::{
    client::IdmClient,
    protocol::{idm_event::Event, idm_packet::Content, IdmEvent, IdmExitEvent, IdmPacket},
};
use log::debug;
use nix::unistd::Pid;
use tokio::select;

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
        loop {
            select! {
                x = self.idm.receiver.recv() => match x {
                    Some(_packet) => {

                    },

                    None => {
                        debug!("idm packet channel closed");
                        break;
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
                .sender
                .send(IdmPacket {
                    content: Some(Content::Event(IdmEvent {
                        event: Some(Event::Exit(IdmExitEvent { code: event.status })),
                    })),
                })
                .await?;
            death(event.status).await?;
        }
        Ok(())
    }
}
