use std::collections::HashMap;
use xencl::{XenClient, XenClientError};

fn main() -> Result<(), XenClientError> {
    let mut client = XenClient::open()?;
    let entries: HashMap<String, String> = HashMap::new();
    client.create(2, entries)?;
    Ok(())
}
