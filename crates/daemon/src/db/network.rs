use crate::db::KrataDatabase;
use advmac::MacAddr6;
use anyhow::Result;
use krata::v1::common::NetworkReservation as ApiNetworkReservation;
use log::error;
use redb::{ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::net::{Ipv4Addr, Ipv6Addr};
use uuid::Uuid;

const NETWORK_RESERVATION_TABLE: TableDefinition<u128, &[u8]> =
    TableDefinition::new("network-reservation");

#[derive(Clone)]
pub struct NetworkReservationStore {
    db: KrataDatabase,
}

impl NetworkReservationStore {
    pub fn open(db: KrataDatabase) -> Result<Self> {
        let write = db.database.begin_write()?;
        let _ = write.open_table(NETWORK_RESERVATION_TABLE);
        write.commit()?;
        Ok(NetworkReservationStore { db })
    }

    pub async fn read(&self, id: Uuid) -> Result<Option<NetworkReservation>> {
        let read = self.db.database.begin_read()?;
        let table = read.open_table(NETWORK_RESERVATION_TABLE)?;
        let Some(entry) = table.get(id.to_u128_le())? else {
            return Ok(None);
        };
        let bytes = entry.value();
        Ok(Some(serde_json::from_slice(bytes)?))
    }

    pub async fn list(&self) -> Result<HashMap<Uuid, NetworkReservation>> {
        enum ListEntry {
            Valid(Uuid, NetworkReservation),
            Invalid(Uuid),
        }
        let mut reservations: HashMap<Uuid, NetworkReservation> = HashMap::new();

        let corruptions = {
            let read = self.db.database.begin_read()?;
            let table = read.open_table(NETWORK_RESERVATION_TABLE)?;
            table
                .iter()?
                .flat_map(|result| {
                    result.map(|(key, value)| {
                        let uuid = Uuid::from_u128_le(key.value());
                        match serde_json::from_slice::<NetworkReservation>(value.value()) {
                            Ok(reservation) => ListEntry::Valid(uuid, reservation),
                            Err(error) => {
                                error!(
                                    "found invalid network reservation in database for uuid {}: {}",
                                    uuid, error
                                );
                                ListEntry::Invalid(uuid)
                            }
                        }
                    })
                })
                .filter_map(|entry| match entry {
                    ListEntry::Valid(uuid, reservation) => {
                        reservations.insert(uuid, reservation);
                        None
                    }

                    ListEntry::Invalid(uuid) => Some(uuid),
                })
                .collect::<Vec<Uuid>>()
        };

        if !corruptions.is_empty() {
            let write = self.db.database.begin_write()?;
            let mut table = write.open_table(NETWORK_RESERVATION_TABLE)?;
            for corruption in corruptions {
                table.remove(corruption.to_u128_le())?;
            }
        }

        Ok(reservations)
    }

    pub async fn update(&self, id: Uuid, entry: NetworkReservation) -> Result<()> {
        let write = self.db.database.begin_write()?;
        {
            let mut table = write.open_table(NETWORK_RESERVATION_TABLE)?;
            let bytes = serde_json::to_vec(&entry)?;
            table.insert(id.to_u128_le(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub async fn remove(&self, id: Uuid) -> Result<()> {
        let write = self.db.database.begin_write()?;
        {
            let mut table = write.open_table(NETWORK_RESERVATION_TABLE)?;
            table.remove(id.to_u128_le())?;
        }
        write.commit()?;
        Ok(())
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct NetworkReservation {
    pub uuid: String,
    pub ipv4: Ipv4Addr,
    pub ipv6: Ipv6Addr,
    pub mac: MacAddr6,
    pub ipv4_prefix: u8,
    pub ipv6_prefix: u8,
    pub gateway_ipv4: Ipv4Addr,
    pub gateway_ipv6: Ipv6Addr,
    pub gateway_mac: MacAddr6,
}

impl From<NetworkReservation> for ApiNetworkReservation {
    fn from(val: NetworkReservation) -> Self {
        ApiNetworkReservation {
            uuid: val.uuid,
            ipv4: format!("{}/{}", val.ipv4, val.ipv4_prefix),
            ipv6: format!("{}/{}", val.ipv6, val.ipv6_prefix),
            mac: val.mac.to_string().to_lowercase().replace('-', ":"),
            gateway_ipv4: format!("{}/{}", val.gateway_ipv4, val.ipv4_prefix),
            gateway_ipv6: format!("{}/{}", val.gateway_ipv6, val.ipv6_prefix),
            gateway_mac: val.gateway_mac.to_string().to_lowercase().replace('-', ":"),
        }
    }
}
