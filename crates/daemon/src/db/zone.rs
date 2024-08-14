use std::collections::HashMap;

use crate::db::KrataDatabase;
use anyhow::Result;
use krata::v1::common::Zone;
use log::error;
use prost::Message;
use redb::{ReadableTable, TableDefinition};
use uuid::Uuid;

const ZONE_TABLE: TableDefinition<u128, &[u8]> = TableDefinition::new("zone");

#[derive(Clone)]
pub struct ZoneStore {
    db: KrataDatabase,
}

impl ZoneStore {
    pub fn open(db: KrataDatabase) -> Result<Self> {
        let write = db.database.begin_write()?;
        let _ = write.open_table(ZONE_TABLE);
        write.commit()?;
        Ok(ZoneStore { db })
    }

    pub async fn read(&self, id: Uuid) -> Result<Option<Zone>> {
        let read = self.db.database.begin_read()?;
        let table = read.open_table(ZONE_TABLE)?;
        let Some(entry) = table.get(id.to_u128_le())? else {
            return Ok(None);
        };
        let bytes = entry.value();
        Ok(Some(Zone::decode(bytes)?))
    }

    pub async fn list(&self) -> Result<HashMap<Uuid, Zone>> {
        let mut zones: HashMap<Uuid, Zone> = HashMap::new();
        let read = self.db.database.begin_read()?;
        let table = read.open_table(ZONE_TABLE)?;
        for result in table.iter()? {
            let (key, value) = result?;
            let uuid = Uuid::from_u128_le(key.value());
            let state = match Zone::decode(value.value()) {
                Ok(state) => state,
                Err(error) => {
                    error!(
                        "found invalid zone state in database for uuid {}: {}",
                        uuid, error
                    );
                    continue;
                }
            };
            zones.insert(uuid, state);
        }
        Ok(zones)
    }

    pub async fn update(&self, id: Uuid, entry: Zone) -> Result<()> {
        let write = self.db.database.begin_write()?;
        {
            let mut table = write.open_table(ZONE_TABLE)?;
            let bytes = entry.encode_to_vec();
            table.insert(id.to_u128_le(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub async fn remove(&self, id: Uuid) -> Result<()> {
        let write = self.db.database.begin_write()?;
        {
            let mut table = write.open_table(ZONE_TABLE)?;
            table.remove(id.to_u128_le())?;
        }
        write.commit()?;
        Ok(())
    }
}
