use super::{ControllerContext, GuestInfo};
use crate::launch::{GuestLaunchRequest, GuestLauncher};
use anyhow::Result;

pub struct ControllerLaunch<'a> {
    context: &'a mut ControllerContext,
}

impl ControllerLaunch<'_> {
    pub fn new(context: &mut ControllerContext) -> ControllerLaunch<'_> {
        ControllerLaunch { context }
    }

    pub async fn perform<'c, 'r>(
        &'c mut self,
        request: GuestLaunchRequest<'r>,
    ) -> Result<GuestInfo> {
        let mut launcher = GuestLauncher::new()?;
        launcher.launch(self.context, request).await
    }
}
