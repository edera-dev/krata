use anyhow::Result;

pub trait IdmSerializable: Sized + Clone + Send + Sync + 'static {
    fn decode(bytes: &[u8]) -> Result<Self>;
    fn encode(&self) -> Result<Vec<u8>>;
}

pub trait IdmRequest: IdmSerializable {
    type Response: IdmSerializable;
}
