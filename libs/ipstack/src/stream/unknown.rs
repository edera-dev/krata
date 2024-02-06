use std::{io::Error, mem, net::IpAddr};

use etherparse::{IpHeader, Ipv4Extensions, Ipv4Header, Ipv6Extensions, Ipv6Header};
use tokio::sync::mpsc::UnboundedSender;

use crate::{
    packet::{NetworkPacket, TransportHeader},
    TTL,
};

pub struct IpStackUnknownTransport {
    src_addr: IpAddr,
    dst_addr: IpAddr,
    payload: Vec<u8>,
    protocol: u8,
    mtu: u16,
    packet_sender: UnboundedSender<NetworkPacket>,
}

impl IpStackUnknownTransport {
    pub fn new(
        src_addr: IpAddr,
        dst_addr: IpAddr,
        payload: Vec<u8>,
        ip: &IpHeader,
        mtu: u16,
        packet_sender: UnboundedSender<NetworkPacket>,
    ) -> Self {
        let protocol = match ip {
            IpHeader::Version4(ip, _) => ip.protocol,
            IpHeader::Version6(ip, _) => ip.next_header,
        };
        IpStackUnknownTransport {
            src_addr,
            dst_addr,
            payload,
            protocol,
            mtu,
            packet_sender,
        }
    }
    pub fn src_addr(&self) -> IpAddr {
        self.src_addr
    }
    pub fn dst_addr(&self) -> IpAddr {
        self.dst_addr
    }
    pub fn payload(&self) -> &[u8] {
        &self.payload
    }
    pub fn ip_protocol(&self) -> u8 {
        self.protocol
    }
    pub async fn send(&self, mut payload: Vec<u8>) -> Result<(), Error> {
        loop {
            let packet = self.create_rev_packet(&mut payload)?;
            self.packet_sender
                .send(packet)
                .map_err(|_| Error::new(std::io::ErrorKind::Other, "send error"))?;
            if payload.is_empty() {
                return Ok(());
            }
        }
    }

    pub fn create_rev_packet(&self, payload: &mut Vec<u8>) -> Result<NetworkPacket, Error> {
        match (self.dst_addr, self.src_addr) {
            (std::net::IpAddr::V4(dst), std::net::IpAddr::V4(src)) => {
                let mut ip_h = Ipv4Header::new(0, TTL, self.protocol, dst.octets(), src.octets());
                let line_buffer = self.mtu.saturating_sub(ip_h.header_len() as u16);

                let p = if payload.len() > line_buffer as usize {
                    payload.drain(0..line_buffer as usize).collect::<Vec<u8>>()
                } else {
                    mem::take(payload)
                };
                ip_h.payload_len = p.len() as u16;
                Ok(NetworkPacket {
                    ip: etherparse::IpHeader::Version4(ip_h, Ipv4Extensions::default()),
                    transport: TransportHeader::Unknown,
                    payload: p,
                })
            }
            (std::net::IpAddr::V6(dst), std::net::IpAddr::V6(src)) => {
                let mut ip_h = Ipv6Header {
                    traffic_class: 0,
                    flow_label: 0,
                    payload_length: 0,
                    next_header: 17,
                    hop_limit: TTL,
                    source: dst.octets(),
                    destination: src.octets(),
                };
                let line_buffer = self.mtu.saturating_sub(ip_h.header_len() as u16);
                payload.truncate(line_buffer as usize);
                ip_h.payload_length = payload.len() as u16;
                let p = if payload.len() > line_buffer as usize {
                    payload.drain(0..line_buffer as usize).collect::<Vec<u8>>()
                } else {
                    mem::take(payload)
                };
                Ok(NetworkPacket {
                    ip: etherparse::IpHeader::Version6(ip_h, Ipv6Extensions::default()),
                    transport: TransportHeader::Unknown,
                    payload: p,
                })
            }
            _ => unreachable!(),
        }
    }
}
