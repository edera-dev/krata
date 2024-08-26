use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use ipnetwork::{Ipv4Network, Ipv6Network};
use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
    sync::Arc,
};
use tokio::sync::RwLock;
use uuid::Uuid;

use crate::db::network::{NetworkReservation, NetworkReservationStore};

#[derive(Default, Clone)]
pub struct NetworkAssignmentState {
    pub ipv4: HashMap<Ipv4Addr, NetworkReservation>,
    pub ipv6: HashMap<Ipv6Addr, NetworkReservation>,
}

#[derive(Clone)]
pub struct NetworkAssignment {
    ipv4_network: Ipv4Network,
    ipv6_network: Ipv6Network,
    gateway_ipv4: Ipv4Addr,
    gateway_ipv6: Ipv6Addr,
    gateway_mac: MacAddr6,
    store: NetworkReservationStore,
    state: Arc<RwLock<NetworkAssignmentState>>,
}

impl NetworkAssignment {
    pub async fn new(
        host_uuid: Uuid,
        ipv4_network: Ipv4Network,
        ipv6_network: Ipv6Network,
        store: NetworkReservationStore,
    ) -> Result<Self> {
        let mut state = NetworkAssignment::fetch_current_state(&store).await?;
        let gateway_reservation = if let Some(reservation) = store.read(Uuid::nil()).await? {
            reservation
        } else {
            NetworkAssignment::allocate(
                &mut state,
                &store,
                Uuid::nil(),
                ipv4_network,
                ipv6_network,
                None,
                None,
                None,
            )
            .await?
        };

        if store.read(host_uuid).await?.is_none() {
            let _ = NetworkAssignment::allocate(
                &mut state,
                &store,
                host_uuid,
                ipv4_network,
                ipv6_network,
                Some(gateway_reservation.gateway_ipv4),
                Some(gateway_reservation.gateway_ipv6),
                Some(gateway_reservation.gateway_mac),
            )
            .await?;
        }

        let assignment = NetworkAssignment {
            ipv4_network,
            ipv6_network,
            gateway_ipv4: gateway_reservation.ipv4,
            gateway_ipv6: gateway_reservation.ipv6,
            gateway_mac: gateway_reservation.mac,
            store,
            state: Arc::new(RwLock::new(state)),
        };
        Ok(assignment)
    }

    async fn fetch_current_state(
        store: &NetworkReservationStore,
    ) -> Result<NetworkAssignmentState> {
        let reservations = store.list().await?;
        let mut state = NetworkAssignmentState::default();
        for reservation in reservations.values() {
            state.ipv4.insert(reservation.ipv4, reservation.clone());
            state.ipv6.insert(reservation.ipv6, reservation.clone());
        }
        Ok(state)
    }

    #[allow(clippy::too_many_arguments)]
    async fn allocate(
        state: &mut NetworkAssignmentState,
        store: &NetworkReservationStore,
        uuid: Uuid,
        ipv4_network: Ipv4Network,
        ipv6_network: Ipv6Network,
        gateway_ipv4: Option<Ipv4Addr>,
        gateway_ipv6: Option<Ipv6Addr>,
        gateway_mac: Option<MacAddr6>,
    ) -> Result<NetworkReservation> {
        let found_ipv4: Option<Ipv4Addr> = ipv4_network
            .iter()
            .filter(|ip| {
                ip.is_private() && !(ip.is_loopback() || ip.is_multicast() || ip.is_broadcast())
            })
            .filter(|ip| {
                let last = ip.octets()[3];
                // filter for IPs ending in .1 to .250 because .250+ can have special meaning
                (1..250).contains(&last)
            })
            .find(|ip| !state.ipv4.contains_key(ip));

        let found_ipv6: Option<Ipv6Addr> = ipv6_network
            .iter()
            .filter(|ip| !ip.is_loopback() && !ip.is_multicast())
            .filter(|ip| {
                let last = ip.octets()[15];
                last > 0
            })
            .find(|ip| !state.ipv6.contains_key(ip));

        let Some(ipv4) = found_ipv4 else {
            return Err(anyhow!(
                "unable to allocate ipv4 address, assigned network is exhausted"
            ));
        };

        let Some(ipv6) = found_ipv6 else {
            return Err(anyhow!(
                "unable to allocate ipv6 address, assigned network is exhausted"
            ));
        };

        let mut mac = MacAddr6::random();
        mac.set_local(true);
        mac.set_multicast(false);

        let reservation = NetworkReservation {
            uuid: uuid.to_string(),
            ipv4,
            ipv6,
            mac,
            ipv4_prefix: ipv4_network.prefix(),
            ipv6_prefix: ipv6_network.prefix(),
            gateway_ipv4: gateway_ipv4.unwrap_or(ipv4),
            gateway_ipv6: gateway_ipv6.unwrap_or(ipv6),
            gateway_mac: gateway_mac.unwrap_or(mac),
        };
        state.ipv4.insert(ipv4, reservation.clone());
        state.ipv6.insert(ipv6, reservation.clone());
        store.update(uuid, reservation.clone()).await?;
        Ok(reservation)
    }

    pub async fn assign(&self, uuid: Uuid) -> Result<NetworkReservation> {
        let mut state = self.state.write().await;
        let reservation = NetworkAssignment::allocate(
            &mut state,
            &self.store,
            uuid,
            self.ipv4_network,
            self.ipv6_network,
            Some(self.gateway_ipv4),
            Some(self.gateway_ipv6),
            Some(self.gateway_mac),
        )
        .await?;
        Ok(reservation)
    }

    pub async fn recall(&self, uuid: Uuid) -> Result<()> {
        let mut state = self.state.write().await;
        self.store.remove(uuid).await?;
        state
            .ipv4
            .retain(|_, reservation| reservation.uuid != uuid.to_string());
        state
            .ipv6
            .retain(|_, reservation| reservation.uuid != uuid.to_string());
        Ok(())
    }

    pub async fn retrieve(&self, uuid: Uuid) -> Result<Option<NetworkReservation>> {
        self.store.read(uuid).await
    }

    pub async fn reload(&self) -> Result<()> {
        let mut state = self.state.write().await;
        let intermediate = NetworkAssignment::fetch_current_state(&self.store).await?;
        *state = intermediate;
        Ok(())
    }

    pub async fn read(&self) -> Result<NetworkAssignmentState> {
        Ok(self.state.read().await.clone())
    }

    pub async fn read_reservations(&self) -> Result<HashMap<Uuid, NetworkReservation>> {
        self.store.list().await
    }
}
