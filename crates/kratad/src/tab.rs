use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tab {
    #[serde(default)]
    pub guests: HashMap<String, TabGuest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TabGuest {
    pub image: String,
    pub mem: u64,
    pub cpus: u32,
    #[serde(default)]
    pub env: HashMap<String, String>,
    #[serde(default)]
    pub run: Vec<String>,
}
