use anyhow::Result;
use krata::{
    control::{GuestInfo, ListResponse, Request, Response},
    stream::ConnectionStreams,
};

use crate::{listen::DaemonRequestHandler, runtime::Runtime};

pub struct ListRequestHandler {}

impl Default for ListRequestHandler {
    fn default() -> Self {
        Self::new()
    }
}

impl ListRequestHandler {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait::async_trait]
impl DaemonRequestHandler for ListRequestHandler {
    fn accepts(&self, request: &Request) -> bool {
        matches!(request, Request::List(_))
    }

    async fn handle(&self, _: ConnectionStreams, runtime: Runtime, _: Request) -> Result<Response> {
        let guests = runtime.list().await?;
        let guests = guests
            .into_iter()
            .map(GuestInfo::from)
            .collect::<Vec<GuestInfo>>();
        Ok(Response::List(ListResponse { guests }))
    }
}
