use anyhow::{anyhow, Result};

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
            .resolve(id)?
            .ok_or_else(|| anyhow!("unable to resolve container: {}", id))?;
        let domid = info.domid;
        let tty = self.context.xen.get_console_path(domid)?;
        let console = XenConsole::new(&tty).await?;
        console.attach().await?;
        Ok(())
    }
}
