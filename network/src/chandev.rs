use log::warn;
// Referenced https://github.com/vi/wgslirpy/blob/master/crates/libwgslirpy/src/channelized_smoltcp_device.rs
use smoltcp::phy::{Checksum, Device};
use tokio::sync::mpsc::Sender;

pub struct ChannelDevice {
    pub mtu: usize,
    pub tx: Sender<Vec<u8>>,
    pub rx: Option<Vec<u8>>,
}

impl ChannelDevice {
    pub fn new(mtu: usize, tx: Sender<Vec<u8>>) -> Self {
        Self { mtu, tx, rx: None }
    }
}

pub struct RxToken(pub Vec<u8>);

impl Device for ChannelDevice {
    type RxToken<'a> = RxToken where Self: 'a;
    type TxToken<'a> = &'a mut ChannelDevice where Self: 'a;

    fn receive(
        &mut self,
        _timestamp: smoltcp::time::Instant,
    ) -> Option<(Self::RxToken<'_>, Self::TxToken<'_>)> {
        self.rx.take().map(|x| (RxToken(x), self))
    }

    fn transmit(&mut self, _timestamp: smoltcp::time::Instant) -> Option<Self::TxToken<'_>> {
        if self.tx.capacity() == 0 {
            warn!("ran out of transmission capacity");
            return None;
        }
        Some(self)
    }

    fn capabilities(&self) -> smoltcp::phy::DeviceCapabilities {
        let mut capabilities = smoltcp::phy::DeviceCapabilities::default();
        capabilities.medium = smoltcp::phy::Medium::Ethernet;
        capabilities.max_transmission_unit = self.mtu;
        capabilities.checksum = smoltcp::phy::ChecksumCapabilities::ignored();
        capabilities.checksum.tcp = Checksum::Tx;
        capabilities.checksum.ipv4 = Checksum::Tx;
        capabilities.checksum.icmpv4 = Checksum::Tx;
        capabilities.checksum.icmpv6 = Checksum::Tx;
        capabilities
    }
}

impl smoltcp::phy::RxToken for RxToken {
    fn consume<R, F>(mut self, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        f(&mut self.0[..])
    }
}

impl<'a> smoltcp::phy::TxToken for &'a mut ChannelDevice {
    fn consume<R, F>(self, len: usize, f: F) -> R
    where
        F: FnOnce(&mut [u8]) -> R,
    {
        let mut buffer = vec![0u8; len];
        let result = f(&mut buffer[..]);
        if let Err(error) = self.tx.try_send(buffer) {
            warn!("failed to transmit packet: {}", error);
        }
        result
    }
}
