use std::time::Duration;

use crate::childwait::{ChildEvent, ChildWait};
use anyhow::Result;
use nix::{libc::c_int, unistd::Pid};
use tokio::{select, time::sleep};
use xenstore::client::{XsdClient, XsdInterface};

pub struct ContainerBackground {
    child: Pid,
    wait: ChildWait,
}

impl ContainerBackground {
    pub async fn new(child: Pid) -> Result<ContainerBackground> {
        Ok(ContainerBackground {
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
            self.death(event.status).await?;
        }
        Ok(())
    }

    async fn death(&mut self, code: c_int) -> Result<()> {
        let store = XsdClient::open().await?;
        store
            .write_string("krata/guest/exit-code", &code.to_string())
            .await?;
        drop(store);
        loop {
            sleep(Duration::from_secs(1)).await;
        }
    }
}
