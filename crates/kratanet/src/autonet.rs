use anyhow::{anyhow, Result};
use smoltcp::wire::{EthernetAddress, Ipv4Cidr, Ipv6Cidr};
use std::{collections::HashMap, str::FromStr};
use uuid::Uuid;
use xenstore::{XsdClient, XsdInterface, XsdTransaction};

pub struct AutoNetworkCollector {
    client: XsdClient,
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

impl AutoNetworkCollector {
    pub async fn new() -> Result<AutoNetworkCollector> {
        Ok(AutoNetworkCollector {
            client: XsdClient::open().await?,
            known: HashMap::new(),
        })
    }

    pub async fn read(&mut self) -> Result<Vec<NetworkMetadata>> {
        let mut networks = Vec::new();
        let tx = self.client.transaction().await?;
        for domid_string in tx.list("/local/domain").await? {
            let Ok(domid) = domid_string.parse::<u32>() else {
                continue;
            };

            let dom_path = format!("/local/domain/{}", domid_string);
            let Some(uuid_string) = tx.read_string(&format!("{}/krata/uuid", dom_path)).await?
            else {
                continue;
            };

            let Ok(uuid) = uuid_string.parse::<Uuid>() else {
                continue;
            };

            let Ok(guest) =
                AutoNetworkCollector::read_network_side(uuid, &tx, &dom_path, "guest").await
            else {
                continue;
            };

            let Ok(gateway) =
                AutoNetworkCollector::read_network_side(uuid, &tx, &dom_path, "gateway").await
            else {
                continue;
            };

            networks.push(NetworkMetadata {
                domid,
                uuid,
                guest,
                gateway,
            });
        }
        tx.commit().await?;
        Ok(networks)
    }

    async fn read_network_side(
        uuid: Uuid,
        tx: &XsdTransaction,
        dom_path: &str,
        side: &str,
    ) -> Result<NetworkSide> {
        let side_path = format!("{}/krata/network/{}", dom_path, side);
        let Some(ipv4) = tx.read_string(&format!("{}/ipv4", side_path)).await? else {
            return Err(anyhow!(
                "krata domain {} is missing {} ipv4 network entry",
                uuid,
                side
            ));
        };

        let Some(ipv6) = tx.read_string(&format!("{}/ipv6", side_path)).await? else {
            return Err(anyhow!(
                "krata domain {} is missing {} ipv6 network entry",
                uuid,
                side
            ));
        };

        let Some(mac) = tx.read_string(&format!("{}/mac", side_path)).await? else {
            return Err(anyhow!(
                "krata domain {} is missing {} mac address entry",
                uuid,
                side
            ));
        };

        let Ok(ipv4) = Ipv4Cidr::from_str(&ipv4) else {
            return Err(anyhow!(
                "krata domain {} has invalid {} ipv4 network cidr entry: {}",
                uuid,
                side,
                ipv4
            ));
        };

        let Ok(ipv6) = Ipv6Cidr::from_str(&ipv6) else {
            return Err(anyhow!(
                "krata domain {} has invalid {} ipv6 network cidr entry: {}",
                uuid,
                side,
                ipv6
            ));
        };

        let Ok(mac) = EthernetAddress::from_str(&mac) else {
            return Err(anyhow!(
                "krata domain {} has invalid {} mac address entry: {}",
                uuid,
                side,
                mac
            ));
        };

        Ok(NetworkSide { ipv4, ipv6, mac })
    }

    pub async fn read_changes(&mut self) -> Result<AutoNetworkChangeset> {
        let mut seen: Vec<Uuid> = Vec::new();
        let mut added: Vec<NetworkMetadata> = Vec::new();
        let mut removed: Vec<NetworkMetadata> = Vec::new();

        for network in self.read().await? {
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

    pub fn mark_unknown(&mut self, uuid: Uuid) -> Result<bool> {
        Ok(self.known.remove(&uuid).is_some())
    }
}
