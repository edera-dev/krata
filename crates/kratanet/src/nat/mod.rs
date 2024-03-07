use anyhow::Result;
use tokio::sync::mpsc::Sender;

use self::handler::NatHandlerFactory;
use self::processor::NatProcessor;
use bytes::BytesMut;
use smoltcp::wire::EthernetAddress;
use smoltcp::wire::IpCidr;
use tokio::task::JoinHandle;

pub mod handler;
pub mod key;
pub mod processor;
pub mod table;

pub struct Nat {
    pub receive_sender: Sender<BytesMut>,
    task: JoinHandle<()>,
}

impl Nat {
    pub fn new(
        mtu: usize,
        factory: Box<dyn NatHandlerFactory>,
        local_mac: EthernetAddress,
        local_cidrs: Vec<IpCidr>,
        transmit_sender: Sender<BytesMut>,
    ) -> Result<Self> {
        let (receive_sender, task) =
            NatProcessor::launch(mtu, factory, local_mac, local_cidrs, transmit_sender)?;
        Ok(Self {
            receive_sender,
            task,
        })
    }
}

impl Drop for Nat {
    fn drop(&mut self) {
        self.task.abort();
    }
}
