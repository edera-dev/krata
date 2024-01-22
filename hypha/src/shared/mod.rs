use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct LaunchInfo {
    pub run: Option<Vec<String>>,
}
