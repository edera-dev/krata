use crate::idm::DaemonIdmHandle;
use crate::zlt::ZoneLookupTable;
use anyhow::Result;
use async_stream::try_stream;
use krata::v1::control::{SnoopIdmReply, SnoopIdmRequest};
use std::pin::Pin;
use tokio_stream::Stream;
use tonic::Status;

pub struct SnoopIdmRpc {
    idm: DaemonIdmHandle,
    zlt: ZoneLookupTable,
}

impl SnoopIdmRpc {
    pub fn new(idm: DaemonIdmHandle, zlt: ZoneLookupTable) -> Self {
        Self { idm, zlt }
    }

    pub async fn process(
        self,
        _request: SnoopIdmRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<SnoopIdmReply, Status>> + Send + 'static>>> {
        let mut messages = self.idm.snoop();
        let zlt = self.zlt.clone();
        let output = try_stream! {
            while let Ok(event) = messages.recv().await {
                let Some(from_uuid) = zlt.lookup_uuid_by_domid(event.from).await else {
                    continue;
                };
                let Some(to_uuid) = zlt.lookup_uuid_by_domid(event.to).await else {
                    continue;
                };
                yield SnoopIdmReply { from: from_uuid.to_string(), to: to_uuid.to_string(), packet: Some(event.packet) };
            }
        };
        Ok(Box::pin(output))
    }
}
