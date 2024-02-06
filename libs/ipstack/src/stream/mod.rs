use std::net::{SocketAddr, SocketAddrV4, SocketAddrV6};

pub use self::tcp::IpStackTcpStream;
pub use self::udp::IpStackUdpStream;
pub use self::unknown::IpStackUnknownTransport;

mod tcb;
mod tcp;
mod udp;
mod unknown;

pub enum IpStackStream {
    Tcp(IpStackTcpStream),
    Udp(IpStackUdpStream),
    UnknownTransport(IpStackUnknownTransport),
    UnknownNetwork(Vec<u8>),
}

impl IpStackStream {
    pub fn local_addr(&self) -> SocketAddr {
        match self {
            IpStackStream::Tcp(tcp) => tcp.local_addr(),
            IpStackStream::Udp(udp) => udp.local_addr(),
            IpStackStream::UnknownNetwork(_) => {
                SocketAddr::V4(SocketAddrV4::new(std::net::Ipv4Addr::new(0, 0, 0, 0), 0))
            }
            IpStackStream::UnknownTransport(unknown) => match unknown.src_addr() {
                std::net::IpAddr::V4(addr) => SocketAddr::V4(SocketAddrV4::new(addr, 0)),
                std::net::IpAddr::V6(addr) => SocketAddr::V6(SocketAddrV6::new(addr, 0, 0, 0)),
            },
        }
    }
    pub fn peer_addr(&self) -> SocketAddr {
        match self {
            IpStackStream::Tcp(tcp) => tcp.peer_addr(),
            IpStackStream::Udp(udp) => udp.peer_addr(),
            IpStackStream::UnknownNetwork(_) => {
                SocketAddr::V4(SocketAddrV4::new(std::net::Ipv4Addr::new(0, 0, 0, 0), 0))
            }
            IpStackStream::UnknownTransport(unknown) => match unknown.dst_addr() {
                std::net::IpAddr::V4(addr) => SocketAddr::V4(SocketAddrV4::new(addr, 0)),
                std::net::IpAddr::V6(addr) => SocketAddr::V6(SocketAddrV6::new(addr, 0, 0, 0)),
            },
        }
    }
}
