use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct LaunchInfo {
    pub run: Option<Vec<String>>,
}
