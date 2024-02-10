use async_trait::async_trait;

use log::warn;

use tokio::sync::mpsc::channel;
use tokio::sync::mpsc::Sender;

use crate::proxynat::udp::ProxyUdpHandler;

use crate::nat::{NatHandler, NatHandlerFactory, NatKey, NatKeyProtocol};

use self::icmp::ProxyIcmpHandler;

mod icmp;
mod udp;

pub struct ProxyNatHandlerFactory {}

impl Default for ProxyNatHandlerFactory {
    fn default() -> Self {
        Self::new()
    }
}

impl ProxyNatHandlerFactory {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NatHandlerFactory for ProxyNatHandlerFactory {
    async fn nat(
        &self,
        key: NatKey,
        tx_sender: Sender<Vec<u8>>,
        reclaim_sender: Sender<NatKey>,
    ) -> Option<Box<dyn NatHandler>> {
        match key.protocol {
            NatKeyProtocol::Udp => {
                let (rx_sender, rx_receiver) = channel::<Vec<u8>>(4);
                let mut handler = ProxyUdpHandler::new(key, rx_sender);

                if let Err(error) = handler.spawn(rx_receiver, tx_sender, reclaim_sender).await {
                    warn!("unable to spawn udp proxy handler: {}", error);
                    None
                } else {
                    Some(Box::new(handler))
                }
            }

            NatKeyProtocol::Icmp => {
                let (rx_sender, rx_receiver) = channel::<Vec<u8>>(4);
                let mut handler = ProxyIcmpHandler::new(key, rx_sender);

                if let Err(error) = handler.spawn(rx_receiver, tx_sender, reclaim_sender).await {
                    warn!("unable to spawn icmp proxy handler: {}", error);
                    None
                } else {
                    Some(Box::new(handler))
                }
            }

            _ => None,
        }
    }
}
