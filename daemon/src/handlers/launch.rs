use anyhow::{anyhow, Result};
use krata::{
    control::{GuestInfo, LaunchResponse, Request, Response},
    stream::ConnectionStreams,
};

use crate::{
    listen::DaemonRequestHandler,
    runtime::{launch::GuestLaunchRequest, Runtime},
};

pub struct LaunchRequestHandler {}

impl Default for LaunchRequestHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl LaunchRequestHandler {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl DaemonRequestHandler for LaunchRequestHandler {
    fn accepts(&self, request: &Request) -> bool {
        matches!(request, Request::Launch(_))
    }

    async fn handle(
        &self,
        _: ConnectionStreams,
        runtime: Runtime,
        request: Request,
    ) -> Result<Response> {
        let launch = match request {
            Request::Launch(launch) => launch,
            _ => return Err(anyhow!("unknown request")),
        };
        let guest: GuestInfo = runtime
            .launch(GuestLaunchRequest {
                image: &launch.image,
                vcpus: launch.vcpus,
                mem: launch.mem,
                env: launch.env,
                run: launch.run,
                debug: false,
            })
            .await?
            .into();
        Ok(Response::Launch(LaunchResponse { guest }))
    }
}
