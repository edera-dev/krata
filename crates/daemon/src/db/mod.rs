use anyhow::Result;
use redb::Database;
use std::path::Path;
use std::sync::Arc;

pub mod ip;
pub mod zone;

#[derive(Clone)]
pub struct KrataDatabase {
    pub database: Arc<Database>,
}

impl KrataDatabase {
    pub fn open(path: &Path) -> Result<Self> {
        let database = Database::create(path)?;
        Ok(KrataDatabase {
            database: Arc::new(database),
        })
    }
}
