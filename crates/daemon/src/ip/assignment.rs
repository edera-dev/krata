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

use crate::db::ip::{IpReservation, IpReservationStore};

#[derive(Default, Clone)]
pub struct IpAssignmentState {
    pub ipv4: HashMap<Ipv4Addr, IpReservation>,
    pub ipv6: HashMap<Ipv6Addr, IpReservation>,
}

#[derive(Clone)]
pub struct IpAssignment {
    ipv4_network: Ipv4Network,
    ipv6_network: Ipv6Network,
    gateway_ipv4: Ipv4Addr,
    gateway_ipv6: Ipv6Addr,
    gateway_mac: MacAddr6,
    store: IpReservationStore,
    state: Arc<RwLock<IpAssignmentState>>,
}

impl IpAssignment {
    pub async fn new(
        host_uuid: Uuid,
        ipv4_network: Ipv4Network,
        ipv6_network: Ipv6Network,
        store: IpReservationStore,
    ) -> Result<Self> {
        let mut state = IpAssignment::fetch_current_state(&store).await?;
        let reservation = if let Some(reservation) = store.read(host_uuid).await? {
            reservation
        } else {
            IpAssignment::allocate(
                &mut state,
                &store,
                host_uuid,
                ipv4_network,
                ipv6_network,
                None,
                None,
                None,
            )
            .await?
        };
        let assignment = IpAssignment {
            ipv4_network,
            ipv6_network,
            gateway_ipv4: reservation.ipv4,
            gateway_ipv6: reservation.ipv6,
            gateway_mac: reservation.gateway_mac,
            store,
            state: Arc::new(RwLock::new(state)),
        };
        Ok(assignment)
    }

    async fn fetch_current_state(store: &IpReservationStore) -> Result<IpAssignmentState> {
        let reservations = store.list().await?;
        let mut state = IpAssignmentState::default();
        for reservation in reservations.values() {
            state.ipv4.insert(reservation.ipv4, reservation.clone());
            state.ipv6.insert(reservation.ipv6, reservation.clone());
        }
        Ok(state)
    }

    #[allow(clippy::too_many_arguments)]
    async fn allocate(
        state: &mut IpAssignmentState,
        store: &IpReservationStore,
        uuid: Uuid,
        ipv4_network: Ipv4Network,
        ipv6_network: Ipv6Network,
        gateway_ipv4: Option<Ipv4Addr>,
        gateway_ipv6: Option<Ipv6Addr>,
        gateway_mac: Option<MacAddr6>,
    ) -> Result<IpReservation> {
        let mut found_ipv4: Option<Ipv4Addr> = None;
        for ip in ipv4_network.iter() {
            if ip.is_loopback() || ip.is_multicast() || ip.is_broadcast() {
                continue;
            }

            if !ip.is_private() {
                continue;
            }

            let last = ip.octets()[3];
            if last == 0 || last > 250 {
                continue;
            }

            if state.ipv4.contains_key(&ip) {
                continue;
            }
            found_ipv4 = Some(ip);
            break;
        }

        let mut found_ipv6: Option<Ipv6Addr> = None;
        for ip in ipv6_network.iter() {
            if ip.is_loopback() || ip.is_multicast() {
                continue;
            }

            if state.ipv6.contains_key(&ip) {
                continue;
            }
            found_ipv6 = Some(ip);
            break;
        }

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
        mac.set_local(false);
        mac.set_multicast(false);

        let reservation = IpReservation {
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

    pub async fn assign(&self, uuid: Uuid) -> Result<IpReservation> {
        let mut state = self.state.write().await;
        let reservation = IpAssignment::allocate(
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

    pub async fn retrieve(&self, uuid: Uuid) -> Result<Option<IpReservation>> {
        self.store.read(uuid).await
    }

    pub async fn reload(&self) -> Result<()> {
        let mut state = self.state.write().await;
        let intermediate = IpAssignment::fetch_current_state(&self.store).await?;
        *state = intermediate;
        Ok(())
    }

    pub async fn read(&self) -> Result<IpAssignmentState> {
        Ok(self.state.read().await.clone())
    }
}
