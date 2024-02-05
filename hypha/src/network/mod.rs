use std::os::fd::AsRawFd;
use std::str::FromStr;

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::{self, RawSocket};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};

pub struct HyphaNetwork {
    pub device: RawSocket,
    pub addresses: Vec<IpCidr>,
}

impl HyphaNetwork {
    pub fn new(iface: &str, cidrs: &[&str]) -> Result<HyphaNetwork> {
        let device = RawSocket::new(iface, smoltcp::phy::Medium::Ethernet)?;
        let mut addresses: Vec<IpCidr> = Vec::new();
        for cidr in cidrs {
            let address =
                IpCidr::from_str(cidr).map_err(|_| anyhow!("failed to parse cidr: {}", *cidr))?;
            addresses.push(address);
        }
        Ok(HyphaNetwork { device, addresses })
    }

    pub fn run(&mut self) -> Result<()> {
        let mac = MacAddr6::random();
        let mac = HardwareAddress::Ethernet(EthernetAddress(mac.to_array()));
        let config = Config::new(mac);
        let mut iface = Interface::new(config, &mut self.device, Instant::now());
        iface.update_ip_addrs(|addrs| {
            addrs
                .extend_from_slice(&self.addresses)
                .expect("failed to set ip addresses");
        });

        let mut sockets = SocketSet::new(vec![]);
        let fd = self.device.as_raw_fd();
        loop {
            let timestamp = Instant::now();
            iface.poll(timestamp, &mut self.device, &mut sockets);
            phy::wait(fd, iface.poll_delay(timestamp, &sockets))?;
        }
    }
}
