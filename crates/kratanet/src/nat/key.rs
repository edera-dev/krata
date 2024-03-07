use std::fmt::Display;

use smoltcp::wire::{EthernetAddress, IpEndpoint};

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub enum NatKeyProtocol {
    Tcp,
    Udp,
    Icmp,
}

#[derive(Debug, Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash)]
pub struct NatKey {
    pub protocol: NatKeyProtocol,
    pub client_mac: EthernetAddress,
    pub local_mac: EthernetAddress,
    pub client_ip: IpEndpoint,
    pub external_ip: IpEndpoint,
}

impl Display for NatKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{} -> {} {:?} {} -> {}",
            self.client_mac, self.local_mac, self.protocol, self.client_ip, self.external_ip
        )
    }
}
