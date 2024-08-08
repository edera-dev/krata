use std::{
    collections::HashMap,
    net::{Ipv4Addr, Ipv6Addr},
    str::FromStr,
    sync::Arc,
};

use anyhow::{anyhow, Result};
use ipnetwork::{Ipv4Network, Ipv6Network};
use log::{debug, error, trace};
use tokio::sync::RwLock;
use uuid::Uuid;
use xenstore::{XsdClient, XsdInterface};

#[derive(Default, Clone)]
pub struct IpVendorState {
    pub ipv4: HashMap<Ipv4Addr, Uuid>,
    pub ipv6: HashMap<Ipv6Addr, Uuid>,
    pub pending_ipv4: HashMap<Ipv4Addr, Uuid>,
    pub pending_ipv6: HashMap<Ipv6Addr, Uuid>,
}

#[derive(Clone)]
pub struct IpVendor {
    store: XsdClient,
    host_uuid: Uuid,
    ipv4_network: Ipv4Network,
    ipv6_network: Ipv6Network,
    gateway_ipv4: Ipv4Addr,
    gateway_ipv6: Ipv6Addr,
    state: Arc<RwLock<IpVendorState>>,
}

pub struct IpAssignment {
    vendor: IpVendor,
    pub uuid: Uuid,
    pub ipv4: Ipv4Addr,
    pub ipv6: Ipv6Addr,
    pub ipv4_prefix: u8,
    pub ipv6_prefix: u8,
    pub gateway_ipv4: Ipv4Addr,
    pub gateway_ipv6: Ipv6Addr,
    pub committed: bool,
}

impl IpAssignment {
    pub async fn commit(&mut self) -> Result<()> {
        self.vendor.commit(self).await?;
        self.committed = true;
        Ok(())
    }
}

impl Drop for IpAssignment {
    fn drop(&mut self) {
        if !self.committed {
            let ipv4 = self.ipv4;
            let ipv6 = self.ipv6;
            let uuid = self.uuid;
            let vendor = self.vendor.clone();
            tokio::task::spawn(async move {
                let _ = vendor.recall_raw(ipv4, ipv6, uuid, true).await;
            });
        }
    }
}

impl IpVendor {
    pub async fn new(
        store: XsdClient,
        host_uuid: Uuid,
        ipv4_network: Ipv4Network,
        ipv6_network: Ipv6Network,
    ) -> Result<Self> {
        debug!("fetching state from xenstore");
        let mut state = IpVendor::fetch_stored_state(&store).await?;
        debug!("allocating IP set");
        let (gateway_ipv4, gateway_ipv6) =
            IpVendor::allocate_ipset(&mut state, host_uuid, ipv4_network, ipv6_network)?;
        let vend = IpVendor {
            store,
            host_uuid,
            ipv4_network,
            ipv6_network,
            gateway_ipv4,
            gateway_ipv6,
            state: Arc::new(RwLock::new(state)),
        };
        debug!("IP vendor initialized!");
        Ok(vend)
    }

    async fn fetch_stored_state(store: &XsdClient) -> Result<IpVendorState> {
        debug!("initializing default IP vendor state");
        let mut state = IpVendorState::default();
        debug!("iterating over xen domains");
        for domid_candidate in store.list("/local/domain").await? {
            let dom_path = format!("/local/domain/{}", domid_candidate);
            let Some(uuid) = store
                .read_string(format!("{}/krata/uuid", dom_path))
                .await?
                .and_then(|x| Uuid::from_str(&x).ok())
            else {
                continue;
            };
            let assigned_ipv4 = store
                .read_string(format!("{}/krata/network/zone/ipv4", dom_path))
                .await?
                .and_then(|x| Ipv4Network::from_str(&x).ok());
            let assigned_ipv6 = store
                .read_string(format!("{}/krata/network/zone/ipv6", dom_path))
                .await?
                .and_then(|x| Ipv6Network::from_str(&x).ok());

            if let Some(existing_ipv4) = assigned_ipv4 {
                if let Some(previous) = state.ipv4.insert(existing_ipv4.ip(), uuid) {
                    error!("ipv4 conflict detected: zone {} owned {} but {} also claimed to own it, giving it to {}", previous, existing_ipv4.ip(), uuid, uuid);
                }
            }

            if let Some(existing_ipv6) = assigned_ipv6 {
                if let Some(previous) = state.ipv6.insert(existing_ipv6.ip(), uuid) {
                    error!("ipv6 conflict detected: zone {} owned {} but {} also claimed to own it, giving it to {}", previous, existing_ipv6.ip(), uuid, uuid);
                }
            }
        }
        debug!("IP state hydrated");
        Ok(state)
    }

    fn allocate_ipset(
        state: &mut IpVendorState,
        uuid: Uuid,
        ipv4_network: Ipv4Network,
        ipv6_network: Ipv6Network,
    ) -> Result<(Ipv4Addr, Ipv6Addr)> {
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

        state.ipv4.insert(ipv4, uuid);
        state.ipv6.insert(ipv6, uuid);

        Ok((ipv4, ipv6))
    }

    pub async fn assign(&self, uuid: Uuid) -> Result<IpAssignment> {
        let mut state = self.state.write().await;
        let (ipv4, ipv6) =
            IpVendor::allocate_ipset(&mut state, uuid, self.ipv4_network, self.ipv6_network)?;
        state.pending_ipv4.insert(ipv4, uuid);
        state.pending_ipv6.insert(ipv6, uuid);
        Ok(IpAssignment {
            vendor: self.clone(),
            uuid,
            ipv4,
            ipv6,
            ipv4_prefix: self.ipv4_network.prefix(),
            ipv6_prefix: self.ipv6_network.prefix(),
            gateway_ipv4: self.gateway_ipv4,
            gateway_ipv6: self.gateway_ipv6,
            committed: false,
        })
    }

    pub async fn commit(&self, assignment: &IpAssignment) -> Result<()> {
        let mut state = self.state.write().await;
        if state.pending_ipv4.remove(&assignment.ipv4) != Some(assignment.uuid) {
            return Err(anyhow!("matching pending ipv4 assignment was not found"));
        }
        if state.pending_ipv6.remove(&assignment.ipv6) != Some(assignment.uuid) {
            return Err(anyhow!("matching pending ipv6 assignment was not found"));
        }
        Ok(())
    }

    async fn recall_raw(
        &self,
        ipv4: Ipv4Addr,
        ipv6: Ipv6Addr,
        uuid: Uuid,
        pending: bool,
    ) -> Result<()> {
        let mut state = self.state.write().await;
        if pending {
            if state.pending_ipv4.remove(&ipv4) != Some(uuid) {
                return Err(anyhow!("matching pending ipv4 assignment was not found"));
            }
            if state.pending_ipv6.remove(&ipv6) != Some(uuid) {
                return Err(anyhow!("matching pending ipv6 assignment was not found"));
            }
        }

        if state.ipv4.remove(&ipv4) != Some(uuid) {
            return Err(anyhow!("matching allocated ipv4 assignment was not found"));
        }

        if state.ipv6.remove(&ipv6) != Some(uuid) {
            return Err(anyhow!("matching allocated ipv6 assignment was not found"));
        }
        Ok(())
    }

    pub async fn recall(&self, assignment: &IpAssignment) -> Result<()> {
        self.recall_raw(assignment.ipv4, assignment.ipv6, assignment.uuid, false)
            .await?;
        Ok(())
    }

    pub async fn reload(&self) -> Result<()> {
        let mut state = self.state.write().await;
        let mut intermediate = IpVendor::fetch_stored_state(&self.store).await?;
        intermediate.ipv4.insert(self.gateway_ipv4, self.host_uuid);
        intermediate.ipv6.insert(self.gateway_ipv6, self.host_uuid);
        for (ipv4, uuid) in &state.pending_ipv4 {
            if let Some(previous) = intermediate.ipv4.insert(*ipv4, *uuid) {
                error!("ipv4 conflict detected: zone {} owned (pending) {} but {} also claimed to own it, giving it to {}", previous, ipv4, uuid, uuid);
            }
            intermediate.pending_ipv4.insert(*ipv4, *uuid);
        }
        for (ipv6, uuid) in &state.pending_ipv6 {
            if let Some(previous) = intermediate.ipv6.insert(*ipv6, *uuid) {
                error!("ipv6 conflict detected: zone {} owned (pending) {} but {} also claimed to own it, giving it to {}", previous, ipv6, uuid, uuid);
            }
            intermediate.pending_ipv6.insert(*ipv6, *uuid);
        }
        *state = intermediate;
        Ok(())
    }

    pub async fn read_domain_assignment(
        &self,
        uuid: Uuid,
        domid: u32,
    ) -> Result<Option<IpAssignment>> {
        let dom_path = format!("/local/domain/{}", domid);
        let Some(zone_ipv4) = self
            .store
            .read_string(format!("{}/krata/network/zone/ipv4", dom_path))
            .await?
        else {
            return Ok(None);
        };
        let Some(zone_ipv6) = self
            .store
            .read_string(format!("{}/krata/network/zone/ipv6", dom_path))
            .await?
        else {
            return Ok(None);
        };
        let Some(gateway_ipv4) = self
            .store
            .read_string(format!("{}/krata/network/gateway/ipv4", dom_path))
            .await?
        else {
            return Ok(None);
        };
        let Some(gateway_ipv6) = self
            .store
            .read_string(format!("{}/krata/network/gateway/ipv6", dom_path))
            .await?
        else {
            return Ok(None);
        };

        let Some(zone_ipv4) = Ipv4Network::from_str(&zone_ipv4).ok() else {
            return Ok(None);
        };
        let Some(zone_ipv6) = Ipv6Network::from_str(&zone_ipv6).ok() else {
            return Ok(None);
        };
        let Some(gateway_ipv4) = Ipv4Network::from_str(&gateway_ipv4).ok() else {
            return Ok(None);
        };
        let Some(gateway_ipv6) = Ipv6Network::from_str(&gateway_ipv6).ok() else {
            return Ok(None);
        };
        Ok(Some(IpAssignment {
            vendor: self.clone(),
            uuid,
            ipv4: zone_ipv4.ip(),
            ipv4_prefix: zone_ipv4.prefix(),
            ipv6: zone_ipv6.ip(),
            ipv6_prefix: zone_ipv6.prefix(),
            gateway_ipv4: gateway_ipv4.ip(),
            gateway_ipv6: gateway_ipv6.ip(),
            committed: true,
        }))
    }

    pub async fn read(&self) -> Result<IpVendorState> {
        Ok(self.state.read().await.clone())
    }
}
