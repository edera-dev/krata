use core::task;
use std::{
    future::Future,
    io::{self, Error, ErrorKind},
    net::SocketAddr,
    pin::Pin,
    task::Poll,
    time::Duration,
};

use etherparse::{Ipv4Extensions, Ipv4Header, Ipv6Extensions, Ipv6Header, UdpHeader};
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::mpsc::{self, UnboundedReceiver, UnboundedSender},
    time::Sleep,
};
// use crate::packet::TransportHeader;
use crate::{
    packet::{NetworkPacket, TransportHeader},
    TTL,
};

pub struct IpStackUdpStream {
    src_addr: SocketAddr,
    dst_addr: SocketAddr,
    stream_sender: UnboundedSender<NetworkPacket>,
    stream_receiver: UnboundedReceiver<NetworkPacket>,
    packet_sender: UnboundedSender<NetworkPacket>,
    first_paload: Option<Vec<u8>>,
    timeout: Pin<Box<Sleep>>,
    udp_timeout: Duration,
    mtu: u16,
}

impl IpStackUdpStream {
    pub fn new(
        src_addr: SocketAddr,
        dst_addr: SocketAddr,
        payload: Vec<u8>,
        pkt_sender: UnboundedSender<NetworkPacket>,
        mtu: u16,
        udp_timeout: Duration,
    ) -> Self {
        let (stream_sender, stream_receiver) = mpsc::unbounded_channel::<NetworkPacket>();
        IpStackUdpStream {
            src_addr,
            dst_addr,
            stream_sender,
            stream_receiver,
            packet_sender: pkt_sender.clone(),
            first_paload: Some(payload),
            timeout: Box::pin(tokio::time::sleep_until(
                tokio::time::Instant::now() + udp_timeout,
            )),
            udp_timeout,
            mtu,
        }
    }
    pub(crate) fn stream_sender(&self) -> UnboundedSender<NetworkPacket> {
        self.stream_sender.clone()
    }
    fn create_rev_packet(&self, ttl: u8, mut payload: Vec<u8>) -> Result<NetworkPacket, Error> {
        match (self.dst_addr.ip(), self.src_addr.ip()) {
            (std::net::IpAddr::V4(dst), std::net::IpAddr::V4(src)) => {
                let mut ip_h = Ipv4Header::new(0, ttl, 17, dst.octets(), src.octets());
                let line_buffer = self.mtu.saturating_sub(ip_h.header_len() as u16 + 8); // 8 is udp header size
                payload.truncate(line_buffer as usize);
                ip_h.payload_len = payload.len() as u16 + 8; // 8 is udp header size
                let udp_header = UdpHeader::with_ipv4_checksum(
                    self.dst_addr.port(),
                    self.src_addr.port(),
                    &ip_h,
                    &payload,
                )
                .map_err(|_e| Error::from(ErrorKind::InvalidInput))?;
                Ok(NetworkPacket {
                    ip: etherparse::IpHeader::Version4(ip_h, Ipv4Extensions::default()),
                    transport: TransportHeader::Udp(udp_header),
                    payload,
                })
            }
            (std::net::IpAddr::V6(dst), std::net::IpAddr::V6(src)) => {
                let mut ip_h = Ipv6Header {
                    traffic_class: 0,
                    flow_label: 0,
                    payload_length: 0,
                    next_header: 17,
                    hop_limit: ttl,
                    source: dst.octets(),
                    destination: src.octets(),
                };
                let line_buffer = self.mtu.saturating_sub(ip_h.header_len() as u16 + 8); // 8 is udp header size

                payload.truncate(line_buffer as usize);

                ip_h.payload_length = payload.len() as u16 + 8; // 8 is udp header size
                let udp_header = UdpHeader::with_ipv6_checksum(
                    self.dst_addr.port(),
                    self.src_addr.port(),
                    &ip_h,
                    &payload,
                )
                .map_err(|_e| Error::from(ErrorKind::InvalidInput))?;
                Ok(NetworkPacket {
                    ip: etherparse::IpHeader::Version6(ip_h, Ipv6Extensions::default()),
                    transport: TransportHeader::Udp(udp_header),
                    payload,
                })
            }
            _ => unreachable!(),
        }
    }
    pub fn local_addr(&self) -> SocketAddr {
        self.src_addr
    }
    pub fn peer_addr(&self) -> SocketAddr {
        self.dst_addr
    }
}

impl AsyncRead for IpStackUdpStream {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut task::Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> task::Poll<io::Result<()>> {
        if let Some(p) = self.first_paload.take() {
            buf.put_slice(&p);
            return Poll::Ready(Ok(()));
        }
        if matches!(self.timeout.as_mut().poll(cx), std::task::Poll::Ready(_)) {
            return Poll::Ready(Ok(())); // todo: return timeout error
        }

        let udp_timeout = self.udp_timeout;
        match self.stream_receiver.poll_recv(cx) {
            Poll::Ready(Some(p)) => {
                buf.put_slice(&p.payload);
                self.timeout
                    .as_mut()
                    .reset(tokio::time::Instant::now() + udp_timeout);
                Poll::Ready(Ok(()))
            }
            Poll::Ready(None) => Poll::Ready(Ok(())),
            Poll::Pending => Poll::Pending,
        }
    }
}

impl AsyncWrite for IpStackUdpStream {
    fn poll_write(
        mut self: Pin<&mut Self>,
        _cx: &mut task::Context<'_>,
        buf: &[u8],
    ) -> task::Poll<Result<usize, io::Error>> {
        let udp_timeout = self.udp_timeout;
        self.timeout
            .as_mut()
            .reset(tokio::time::Instant::now() + udp_timeout);
        let packet = self.create_rev_packet(TTL, buf.to_vec())?;
        let payload_len = packet.payload.len();
        self.packet_sender
            .send(packet)
            .map_err(|_| Error::from(ErrorKind::UnexpectedEof))?;
        std::task::Poll::Ready(Ok(payload_len))
    }

    fn poll_flush(
        self: Pin<&mut Self>,
        _cx: &mut task::Context<'_>,
    ) -> task::Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(
        self: Pin<&mut Self>,
        _cx: &mut task::Context<'_>,
    ) -> task::Poll<Result<(), io::Error>> {
        Poll::Ready(Ok(()))
    }
}
