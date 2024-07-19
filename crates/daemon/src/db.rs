use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use krata::v1::common::Zone;
use log::error;
use prost::Message;
use redb::{Database, ReadableTable, TableDefinition};
use uuid::Uuid;

const ZONES: TableDefinition<u128, &[u8]> = TableDefinition::new("zones");

#[derive(Clone)]
pub struct ZoneStore {
    database: Arc<Database>,
}

impl ZoneStore {
    pub fn open(path: &Path) -> Result<Self> {
        let database = Database::create(path)?;
        let write = database.begin_write()?;
        let _ = write.open_table(ZONES);
        write.commit()?;
        Ok(ZoneStore {
            database: Arc::new(database),
        })
    }

    pub async fn read(&self, id: Uuid) -> Result<Option<Zone>> {
        let read = self.database.begin_read()?;
        let table = read.open_table(ZONES)?;
        let Some(entry) = table.get(id.to_u128_le())? else {
            return Ok(None);
        };
        let bytes = entry.value();
        Ok(Some(Zone::decode(bytes)?))
    }

    pub async fn list(&self) -> Result<HashMap<Uuid, Zone>> {
        let mut zones: HashMap<Uuid, Zone> = HashMap::new();
        let read = self.database.begin_read()?;
        let table = read.open_table(ZONES)?;
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
        let write = self.database.begin_write()?;
        {
            let mut table = write.open_table(ZONES)?;
            let bytes = entry.encode_to_vec();
            table.insert(id.to_u128_le(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub async fn remove(&self, id: Uuid) -> Result<()> {
        let write = self.database.begin_write()?;
        {
            let mut table = write.open_table(ZONES)?;
            table.remove(id.to_u128_le())?;
        }
        write.commit()?;
        Ok(())
    }
}
