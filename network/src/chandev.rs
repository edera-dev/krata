use bytes::BytesMut;
// Referenced https://github.com/vi/wgslirpy/blob/master/crates/libwgslirpy/src/channelized_smoltcp_device.rs
use log::{debug, warn};
use smoltcp::phy::{Checksum, Device, Medium};
use tokio::sync::mpsc::Sender;

const TEAR_OFF_BUFFER_SIZE: usize = 65536;

pub struct ChannelDevice {
    pub mtu: usize,
    pub medium: Medium,
    pub tx: Sender<BytesMut>,
    pub rx: Option<BytesMut>,
    tear_off_buffer: BytesMut,
}

impl ChannelDevice {
    pub fn new(mtu: usize, medium: Medium, tx: Sender<BytesMut>) -> Self {
        Self {
            mtu,
            medium,
            tx,
            rx: None,
            tear_off_buffer: BytesMut::with_capacity(TEAR_OFF_BUFFER_SIZE),
        }
    }
}

pub struct RxToken(pub BytesMut);

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
            debug!("ran out of transmission capacity");
            return None;
        }
        Some(self)
    }

    fn capabilities(&self) -> smoltcp::phy::DeviceCapabilities {
        let mut capabilities = smoltcp::phy::DeviceCapabilities::default();
        capabilities.medium = self.medium;
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
        self.tear_off_buffer.resize(len, 0);
        let result = f(&mut self.tear_off_buffer[..]);
        let chunk = self.tear_off_buffer.split();
        if let Err(error) = self.tx.try_send(chunk) {
            warn!("failed to transmit packet: {}", error);
        }

        if self.tear_off_buffer.capacity() < self.mtu {
            self.tear_off_buffer = BytesMut::with_capacity(TEAR_OFF_BUFFER_SIZE);
        }
        result
    }
}
