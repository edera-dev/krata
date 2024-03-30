use std::{collections::HashMap, path::Path, sync::Arc};

use anyhow::Result;
use krata::v1::common::Guest;
use log::error;
use prost::Message;
use redb::{Database, ReadableTable, TableDefinition};
use uuid::Uuid;

const GUESTS: TableDefinition<u128, &[u8]> = TableDefinition::new("guests");

#[derive(Clone)]
pub struct GuestStore {
    database: Arc<Database>,
}

impl GuestStore {
    pub fn open(path: &Path) -> Result<Self> {
        let database = Database::create(path)?;
        let write = database.begin_write()?;
        let _ = write.open_table(GUESTS);
        write.commit()?;
        Ok(GuestStore {
            database: Arc::new(database),
        })
    }

    pub async fn read(&self, id: Uuid) -> Result<Option<Guest>> {
        let read = self.database.begin_read()?;
        let table = read.open_table(GUESTS)?;
        let Some(entry) = table.get(id.to_u128_le())? else {
            return Ok(None);
        };
        let bytes = entry.value();
        Ok(Some(Guest::decode(bytes)?))
    }

    pub async fn list(&self) -> Result<HashMap<Uuid, Guest>> {
        let mut guests: HashMap<Uuid, Guest> = HashMap::new();
        let read = self.database.begin_read()?;
        let table = read.open_table(GUESTS)?;
        for result in table.iter()? {
            let (key, value) = result?;
            let uuid = Uuid::from_u128_le(key.value());
            let state = match Guest::decode(value.value()) {
                Ok(state) => state,
                Err(error) => {
                    error!(
                        "found invalid guest state in database for uuid {}: {}",
                        uuid, error
                    );
                    continue;
                }
            };
            guests.insert(uuid, state);
        }
        Ok(guests)
    }

    pub async fn update(&self, id: Uuid, entry: Guest) -> Result<()> {
        let write = self.database.begin_write()?;
        {
            let mut table = write.open_table(GUESTS)?;
            let bytes = entry.encode_to_vec();
            table.insert(id.to_u128_le(), bytes.as_slice())?;
        }
        write.commit()?;
        Ok(())
    }

    pub async fn remove(&self, id: Uuid) -> Result<()> {
        let write = self.database.begin_write()?;
        {
            let mut table = write.open_table(GUESTS)?;
            table.remove(id.to_u128_le())?;
        }
        write.commit()?;
        Ok(())
    }
}
