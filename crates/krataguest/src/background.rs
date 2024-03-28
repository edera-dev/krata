use crate::{
    childwait::{ChildEvent, ChildWait},
    death,
};
use anyhow::Result;
use krata::idm::{
    client::IdmClient,
    protocol::{idm_packet::Message, IdmExitMessage, IdmPacket},
};
use log::error;
use nix::unistd::Pid;
use tokio::select;

pub struct GuestBackground {
    idm: IdmClient,
    child: Pid,
    wait: ChildWait,
}

impl GuestBackground {
    pub async fn new(idm: IdmClient, child: Pid) -> Result<GuestBackground> {
        Ok(GuestBackground {
            idm,
            child,
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
                        error!("idm packet channel closed");
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
                    message: Some(Message::Exit(IdmExitMessage { code: event.status })),
                })
                .await?;
            death(event.status).await?;
        }
        Ok(())
    }
}
