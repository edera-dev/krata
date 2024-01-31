use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchInfo {
    pub env: Option<Vec<String>>,
    pub run: Option<Vec<String>>,
}
