use anyhow::Result;
use async_trait::async_trait;
use bytes::BytesMut;
use tokio::sync::mpsc::Sender;

use super::key::NatKey;

#[derive(Debug, Clone)]
pub struct NatHandlerContext {
    pub mtu: usize,
    pub key: NatKey,
    pub transmit_sender: Sender<BytesMut>,
    pub reclaim_sender: Sender<NatKey>,
}

impl NatHandlerContext {
    pub fn try_transmit(&self, buffer: BytesMut) -> Result<()> {
        self.transmit_sender.try_send(buffer)?;
        Ok(())
    }

    pub async fn reclaim(&self) -> Result<()> {
        self.reclaim_sender.try_send(self.key)?;
        Ok(())
    }
}

#[async_trait]
pub trait NatHandler: Send {
    async fn receive(&self, packet: &[u8]) -> Result<bool>;
}

#[async_trait]
pub trait NatHandlerFactory: Send {
    async fn nat(&self, context: NatHandlerContext) -> Option<Box<dyn NatHandler>>;
}
