mod udp;

use async_trait::async_trait;

use log::{debug, warn};

use tokio::sync::mpsc::channel;
use tokio::sync::mpsc::Sender;

use crate::proxynat::udp::ProxyUdpHandler;

use crate::nat::{NatHandler, NatHandlerFactory, NatKey, NatKeyProtocol};

pub struct ProxyNatHandlerFactory {}

impl ProxyNatHandlerFactory {
    pub fn new() -> Self {
        Self {}
    }
}

#[async_trait]
impl NatHandlerFactory for ProxyNatHandlerFactory {
    async fn nat(&self, key: NatKey, sender: Sender<Vec<u8>>) -> Option<Box<dyn NatHandler>> {
        debug!("creating proxy nat entry for key: {}", key);

        match key.protocol {
            NatKeyProtocol::Udp => {
                let (rx_sender, rx_receiver) = channel::<Vec<u8>>(4);
                let mut handler = ProxyUdpHandler::new(key, rx_sender);

                if let Err(error) = handler.spawn(rx_receiver, sender.clone()).await {
                    warn!("unable to spawn udp proxy handler: {}", error);
                    None
                } else {
                    Some(Box::new(handler))
                }
            }

            _ => None,
        }
    }
}

pub enum ProxyNatSelect {
    External(usize),
    Internal(Vec<u8>),
    Closed,
}
