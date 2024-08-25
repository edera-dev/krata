use std::str::FromStr;

use anyhow::Result;
use uuid::Uuid;

use krata::idm::internal::MetricsRequest;
use krata::idm::internal::{
    request::Request as IdmRequestType, response::Response as IdmResponseType,
    Request as IdmRequest,
};
use krata::v1::control::{ReadZoneMetricsReply, ReadZoneMetricsRequest};

use crate::idm::DaemonIdmHandle;
use crate::metrics::idm_metric_to_api;

pub struct ReadZoneMetricsRpc {
    idm: DaemonIdmHandle,
}

impl ReadZoneMetricsRpc {
    pub fn new(idm: DaemonIdmHandle) -> Self {
        Self { idm }
    }

    pub async fn process(self, request: ReadZoneMetricsRequest) -> Result<ReadZoneMetricsReply> {
        let uuid = Uuid::from_str(&request.zone_id)?;
        let client = self.idm.client(uuid).await?;
        let response = client
            .send(IdmRequest {
                request: Some(IdmRequestType::Metrics(MetricsRequest {})),
            })
            .await?;

        let mut reply = ReadZoneMetricsReply::default();
        if let Some(IdmResponseType::Metrics(metrics)) = response.response {
            reply.root = metrics.root.map(idm_metric_to_api);
        }
        Ok(reply)
    }
}
