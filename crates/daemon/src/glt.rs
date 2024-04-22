use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;
use uuid::Uuid;

struct GuestLookupTableState {
    domid_to_uuid: HashMap<u32, Uuid>,
    uuid_to_domid: HashMap<Uuid, u32>,
}

impl GuestLookupTableState {
    pub fn new(host_uuid: Uuid) -> Self {
        let mut domid_to_uuid = HashMap::new();
        let mut uuid_to_domid = HashMap::new();
        domid_to_uuid.insert(0, host_uuid);
        uuid_to_domid.insert(host_uuid, 0);
        GuestLookupTableState {
            domid_to_uuid,
            uuid_to_domid,
        }
    }
}

#[derive(Clone)]
pub struct GuestLookupTable {
    host_domid: u32,
    host_uuid: Uuid,
    state: Arc<RwLock<GuestLookupTableState>>,
}

impl GuestLookupTable {
    pub fn new(host_domid: u32, host_uuid: Uuid) -> Self {
        GuestLookupTable {
            host_domid,
            host_uuid,
            state: Arc::new(RwLock::new(GuestLookupTableState::new(host_uuid))),
        }
    }

    pub fn host_uuid(&self) -> Uuid {
        self.host_uuid
    }

    pub fn host_domid(&self) -> u32 {
        self.host_domid
    }

    pub async fn lookup_uuid_by_domid(&self, domid: u32) -> Option<Uuid> {
        let state = self.state.read().await;
        state.domid_to_uuid.get(&domid).cloned()
    }

    pub async fn lookup_domid_by_uuid(&self, uuid: &Uuid) -> Option<u32> {
        let state = self.state.read().await;
        state.uuid_to_domid.get(uuid).cloned()
    }

    pub async fn associate(&self, uuid: Uuid, domid: u32) {
        let mut state = self.state.write().await;
        state.uuid_to_domid.insert(uuid, domid);
        state.domid_to_uuid.insert(domid, uuid);
    }

    pub async fn remove(&self, uuid: Uuid, domid: u32) {
        let mut state = self.state.write().await;
        state.uuid_to_domid.remove(&uuid);
        state.domid_to_uuid.remove(&domid);
    }
}
