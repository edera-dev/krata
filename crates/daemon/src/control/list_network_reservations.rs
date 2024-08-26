use anyhow::Result;

use krata::v1::{
    common::NetworkReservation,
    control::{ListNetworkReservationsReply, ListNetworkReservationsRequest},
};

use crate::network::assignment::NetworkAssignment;

pub struct ListNetworkReservationsRpc {
    network: NetworkAssignment,
}

impl ListNetworkReservationsRpc {
    pub fn new(network: NetworkAssignment) -> Self {
        Self { network }
    }

    pub async fn process(
        self,
        _request: ListNetworkReservationsRequest,
    ) -> Result<ListNetworkReservationsReply> {
        let state = self.network.read_reservations().await?;
        let reservations: Vec<NetworkReservation> =
            state.into_values().map(|x| x.into()).collect::<Vec<_>>();
        Ok(ListNetworkReservationsReply { reservations })
    }
}
