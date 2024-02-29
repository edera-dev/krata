use std::{process::exit, time::Duration};

use anyhow::{anyhow, Result};
use log::warn;
use tokio::time::sleep;
use xenstore::client::XsdInterface;

use super::destroy::ControllerDestroy;
use crate::console::XenConsole;

use super::ControllerContext;

pub struct ControllerConsole<'a> {
    context: &'a mut ControllerContext,
}

impl ControllerConsole<'_> {
    pub fn new(context: &mut ControllerContext) -> ControllerConsole<'_> {
        ControllerConsole { context }
    }

    pub async fn perform(&mut self, id: &str) -> Result<()> {
        let info = self
            .context
            .resolve(id)
            .await?
            .ok_or_else(|| anyhow!("unable to resolve guest: {}", id))?;
        let domid = info.domid;
        let tty = self.context.xen.get_console_path(domid).await?;
        let console = XenConsole::new(&tty).await?;

        let dom_path = self.context.xen.store.get_domain_path(domid).await?;

        tokio::task::spawn(async move {
            if let Err(error) = console.attach().await {
                warn!("failed to attach to console: {}", error);
            }
        });

        let exit_code_path = format!("{}/krata/guest/exit-code", dom_path);
        loop {
            let Some(code) = self.context.xen.store.read_string(&exit_code_path).await? else {
                sleep(Duration::from_secs(1)).await;
                continue;
            };
            let mut destroy = ControllerDestroy::new(self.context);
            destroy.perform(&domid.to_string()).await?;
            exit(code.parse::<i32>()?);
        }
    }
}
