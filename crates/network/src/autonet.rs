use anyhow::Result;
use krata::{
    events::EventStream,
    v1::{
        common::Guest,
        control::{
            control_service_client::ControlServiceClient, watch_events_reply::Event,
            ListGuestsRequest,
        },
    },
};
use log::warn;
use smoltcp::wire::{EthernetAddress, Ipv4Cidr, Ipv6Cidr};
use std::{collections::HashMap, str::FromStr, time::Duration};
use tokio::{select, sync::broadcast::Receiver, time::sleep};
use tonic::transport::Channel;
use uuid::Uuid;

pub struct AutoNetworkWatcher {
    control: ControlServiceClient<Channel>,
    pub events: EventStream,
    known: HashMap<Uuid, NetworkMetadata>,
}

#[derive(Debug, Clone)]
pub struct NetworkSide {
    pub ipv4: Ipv4Cidr,
    pub ipv6: Ipv6Cidr,
    pub mac: EthernetAddress,
}

#[derive(Debug, Clone)]
pub struct NetworkMetadata {
    pub domid: u32,
    pub uuid: Uuid,
    pub guest: NetworkSide,
    pub gateway: NetworkSide,
}

impl NetworkMetadata {
    pub fn interface(&self) -> String {
        format!("vif{}.20", self.domid)
    }
}

#[derive(Debug, Clone)]
pub struct AutoNetworkChangeset {
    pub added: Vec<NetworkMetadata>,
    pub removed: Vec<NetworkMetadata>,
}

impl AutoNetworkWatcher {
    pub async fn new(control: ControlServiceClient<Channel>) -> Result<AutoNetworkWatcher> {
        let client = control.clone();
        Ok(AutoNetworkWatcher {
            control,
            events: EventStream::open(client).await?,
            known: HashMap::new(),
        })
    }

    pub async fn read(&mut self) -> Result<Vec<NetworkMetadata>> {
        let mut all_guests: HashMap<Uuid, Guest> = HashMap::new();
        for guest in self
            .control
            .list_guests(ListGuestsRequest {})
            .await?
            .into_inner()
            .guests
        {
            let Ok(uuid) = Uuid::from_str(&guest.id) else {
                continue;
            };
            all_guests.insert(uuid, guest);
        }

        let mut networks: Vec<NetworkMetadata> = Vec::new();
        for (uuid, guest) in &all_guests {
            let Some(ref state) = guest.state else {
                continue;
            };

            if state.domid == u32::MAX {
                continue;
            }

            let Some(ref network) = state.network else {
                continue;
            };

            let Ok(guest_ipv4_cidr) = Ipv4Cidr::from_str(&network.guest_ipv4) else {
                continue;
            };

            let Ok(guest_ipv6_cidr) = Ipv6Cidr::from_str(&network.guest_ipv6) else {
                continue;
            };

            let Ok(guest_mac) = EthernetAddress::from_str(&network.guest_mac) else {
                continue;
            };

            let Ok(gateway_ipv4_cidr) = Ipv4Cidr::from_str(&network.gateway_ipv4) else {
                continue;
            };

            let Ok(gateway_ipv6_cidr) = Ipv6Cidr::from_str(&network.gateway_ipv6) else {
                continue;
            };

            let Ok(gateway_mac) = EthernetAddress::from_str(&network.gateway_mac) else {
                continue;
            };

            networks.push(NetworkMetadata {
                domid: state.domid,
                uuid: *uuid,
                guest: NetworkSide {
                    ipv4: guest_ipv4_cidr,
                    ipv6: guest_ipv6_cidr,
                    mac: guest_mac,
                },
                gateway: NetworkSide {
                    ipv4: gateway_ipv4_cidr,
                    ipv6: gateway_ipv6_cidr,
                    mac: gateway_mac,
                },
            });
        }
        Ok(networks)
    }

    pub async fn read_changes(&mut self) -> Result<AutoNetworkChangeset> {
        let mut seen: Vec<Uuid> = Vec::new();
        let mut added: Vec<NetworkMetadata> = Vec::new();
        let mut removed: Vec<NetworkMetadata> = Vec::new();

        let networks = match self.read().await {
            Ok(networks) => networks,
            Err(error) => {
                warn!("failed to read network changes: {}", error);
                return Ok(AutoNetworkChangeset { added, removed });
            }
        };

        for network in networks {
            seen.push(network.uuid);
            if self.known.contains_key(&network.uuid) {
                continue;
            }
            let _ = self.known.insert(network.uuid, network.clone());
            added.push(network);
        }

        let mut gone: Vec<Uuid> = Vec::new();
        for uuid in self.known.keys() {
            if seen.contains(uuid) {
                continue;
            }
            gone.push(*uuid);
        }

        for uuid in &gone {
            let Some(network) = self.known.remove(uuid) else {
                continue;
            };

            removed.push(network);
        }

        Ok(AutoNetworkChangeset { added, removed })
    }

    pub async fn wait(&mut self, receiver: &mut Receiver<Event>) -> Result<()> {
        loop {
            select! {
                x = receiver.recv() => match x {
                    Ok(Event::GuestChanged(_)) => {
                        break;
                    },

                    Err(error) => {
                        warn!("failed to receive event: {}", error);
                    }
                },

                _ = sleep(Duration::from_secs(10)) => {
                    break;
                }
            };
        }
        Ok(())
    }

    pub fn mark_unknown(&mut self, uuid: Uuid) -> Result<bool> {
        Ok(self.known.remove(&uuid).is_some())
    }
}
