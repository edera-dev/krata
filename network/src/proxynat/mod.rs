use async_trait::async_trait;

use log::warn;

use tokio::sync::mpsc::channel;

use crate::nat::NatHandlerContext;
use crate::proxynat::udp::ProxyUdpHandler;

use crate::nat::{NatHandler, NatHandlerFactory, NatKeyProtocol};

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
    async fn nat(&self, context: NatHandlerContext) -> Option<Box<dyn NatHandler>> {
        match context.key.protocol {
            NatKeyProtocol::Udp => {
                let (rx_sender, rx_receiver) = channel::<Vec<u8>>(4);
                let mut handler = ProxyUdpHandler::new(rx_sender);

                if let Err(error) = handler.spawn(context, rx_receiver).await {
                    warn!("unable to spawn udp proxy handler: {}", error);
                    None
                } else {
                    Some(Box::new(handler))
                }
            }

            NatKeyProtocol::Icmp => {
                let (rx_sender, rx_receiver) = channel::<Vec<u8>>(4);
                let mut handler = ProxyIcmpHandler::new(rx_sender);

                if let Err(error) = handler.spawn(context, rx_receiver).await {
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
