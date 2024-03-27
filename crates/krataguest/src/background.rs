use crate::{
    childwait::{ChildEvent, ChildWait},
    death,
};
use anyhow::Result;
use nix::unistd::Pid;
use tokio::select;

pub struct GuestBackground {
    child: Pid,
    wait: ChildWait,
}

impl GuestBackground {
    pub async fn new(child: Pid) -> Result<GuestBackground> {
        Ok(GuestBackground {
            child,
            wait: ChildWait::new()?,
        })
    }

    pub async fn run(&mut self) -> Result<()> {
        loop {
            select! {
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
            death(event.status).await?;
        }
        Ok(())
    }
}
