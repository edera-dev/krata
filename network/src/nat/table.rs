use std::collections::HashMap;

use super::{handler::NatHandler, key::NatKey};

pub struct NatTable {
    pub inner: HashMap<NatKey, Box<dyn NatHandler>>,
}

impl Default for NatTable {
    fn default() -> Self {
        Self::new()
    }
}

impl NatTable {
    pub fn new() -> Self {
        Self {
            inner: HashMap::new(),
        }
    }
}
