use anyhow::{anyhow, Result};
use krata::{
    control::{DestroyResponse, Request, Response},
    stream::ConnectionStreams,
};

use crate::{listen::DaemonRequestHandler, runtime::Runtime};

pub struct DestroyRequestHandler {}

impl Default for DestroyRequestHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl DestroyRequestHandler {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl DaemonRequestHandler for DestroyRequestHandler {
    fn accepts(&self, request: &Request) -> bool {
        matches!(request, Request::Destroy(_))
    }

    async fn handle(
        &self,
        _: ConnectionStreams,
        runtime: Runtime,
        request: Request,
    ) -> Result<Response> {
        let destroy = match request {
            Request::Destroy(destroy) => destroy,
            _ => return Err(anyhow!("unknown request")),
        };
        let guest = runtime.destroy(&destroy.guest).await?;
        Ok(Response::Destroy(DestroyResponse {
            guest: guest.to_string(),
        }))
    }
}
