use anyhow::Result;
use etherparse::{Ethernet2Slice, Ipv4Slice, Ipv6Slice, LinkSlice, NetSlice, SlicedPacket};

pub enum RecvPacketIp<'a> {
    Ipv4(&'a Ipv4Slice<'a>),
    Ipv6(&'a Ipv6Slice<'a>),
}

pub struct RecvPacket<'a> {
    pub raw: &'a [u8],
    pub slice: &'a SlicedPacket<'a>,
    pub ether: Option<&'a Ethernet2Slice<'a>>,
    pub ip: Option<RecvPacketIp<'a>>,
}

impl RecvPacket<'_> {
    pub fn new<'a>(raw: &'a [u8], slice: &'a SlicedPacket<'a>) -> Result<RecvPacket<'a>> {
        let ether = match slice.link {
            Some(LinkSlice::Ethernet2(ref ether)) => Some(ether),
            _ => None,
        };

        let ip = match slice.net {
            Some(NetSlice::Ipv4(ref ipv4)) => Some(RecvPacketIp::Ipv4(ipv4)),
            Some(NetSlice::Ipv6(ref ipv6)) => Some(RecvPacketIp::Ipv6(ipv6)),
            _ => None,
        };

        let packet = RecvPacket {
            raw,
            slice,
            ether,
            ip,
        };
        Ok(packet)
    }
}
