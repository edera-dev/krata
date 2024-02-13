use crate::raw_socket::{RawSocketHandle, RawSocketProtocol};
use anyhow::{anyhow, Result};
use etherparse::{
    IcmpEchoHeader, Icmpv4Header, Icmpv4Slice, Icmpv4Type, Icmpv6Header, Icmpv6Slice, Icmpv6Type,
    IpNumber, NetSlice, SlicedPacket,
};
use log::warn;
use std::{
    collections::HashMap,
    net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, SocketAddrV4, SocketAddrV6},
    os::fd::{FromRawFd, IntoRawFd},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::UdpSocket,
    sync::{oneshot, Mutex},
    task::JoinHandle,
    time::timeout,
};

#[derive(Debug)]
pub enum IcmpProtocol {
    Icmpv4,
    Icmpv6,
}

impl IcmpProtocol {
    pub fn to_socket_protocol(&self) -> RawSocketProtocol {
        match self {
            IcmpProtocol::Icmpv4 => RawSocketProtocol::Icmpv4,
            IcmpProtocol::Icmpv6 => RawSocketProtocol::Icmpv6,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct IcmpHandlerToken(IpAddr, Option<u16>, u16);

#[derive(Debug)]
pub enum IcmpReply {
    Icmpv4 {
        header: Icmpv4Header,
        echo: IcmpEchoHeader,
        payload: Vec<u8>,
    },

    Icmpv6 {
        header: Icmpv6Header,
        echo: IcmpEchoHeader,
        payload: Vec<u8>,
    },
}

type IcmpHandlerMap = Arc<Mutex<HashMap<IcmpHandlerToken, oneshot::Sender<IcmpReply>>>>;

#[derive(Clone)]
pub struct IcmpClient {
    socket: Arc<UdpSocket>,
    handlers: IcmpHandlerMap,
    task: Arc<JoinHandle<Result<()>>>,
}

impl IcmpClient {
    pub fn new(protocol: IcmpProtocol) -> Result<IcmpClient> {
        let handle = RawSocketHandle::new(protocol.to_socket_protocol())?;
        let socket = unsafe { std::net::UdpSocket::from_raw_fd(handle.into_raw_fd()) };
        let socket: Arc<UdpSocket> = Arc::new(socket.try_into()?);
        let handlers = Arc::new(Mutex::new(HashMap::new()));
        let task = Arc::new(tokio::task::spawn(IcmpClient::process(
            protocol,
            socket.clone(),
            handlers.clone(),
        )));
        Ok(IcmpClient {
            socket,
            handlers,
            task,
        })
    }

    async fn process(
        protocol: IcmpProtocol,
        socket: Arc<UdpSocket>,
        handlers: IcmpHandlerMap,
    ) -> Result<()> {
        let mut buffer = vec![0u8; 2048];
        loop {
            let (size, addr) = socket.recv_from(&mut buffer).await?;
            let packet = &buffer[0..size];

            let (token, reply) = match protocol {
                IcmpProtocol::Icmpv4 => {
                    let sliced = match SlicedPacket::from_ip(packet) {
                        Ok(sliced) => sliced,
                        Err(error) => {
                            warn!("received icmp packet but failed to parse it: {}", error);
                            continue;
                        }
                    };

                    let Some(NetSlice::Ipv4(ipv4)) = sliced.net else {
                        continue;
                    };

                    if ipv4.header().protocol() != IpNumber::ICMP {
                        continue;
                    }

                    let Ok(icmpv4) = Icmpv4Slice::from_slice(ipv4.payload().payload) else {
                        continue;
                    };

                    let Icmpv4Type::EchoReply(echo) = icmpv4.header().icmp_type else {
                        continue;
                    };

                    let token = IcmpHandlerToken(
                        IpAddr::V4(ipv4.header().source_addr()),
                        Some(echo.id),
                        echo.seq,
                    );
                    let reply = IcmpReply::Icmpv4 {
                        header: icmpv4.header(),
                        echo,
                        payload: icmpv4.payload().to_vec(),
                    };
                    (token, reply)
                }

                IcmpProtocol::Icmpv6 => {
                    let Ok(icmpv6) = Icmpv6Slice::from_slice(packet) else {
                        continue;
                    };

                    let Icmpv6Type::EchoReply(echo) = icmpv6.header().icmp_type else {
                        continue;
                    };

                    let SocketAddr::V6(addr) = addr else {
                        continue;
                    };

                    let token = IcmpHandlerToken(IpAddr::V6(*addr.ip()), Some(echo.id), echo.seq);

                    let reply = IcmpReply::Icmpv6 {
                        header: icmpv6.header(),
                        echo,
                        payload: icmpv6.payload().to_vec(),
                    };
                    (token, reply)
                }
            };

            if let Some(sender) = handlers.lock().await.remove(&token) {
                let _ = sender.send(reply);
            }
        }
    }

    async fn add_handler(&self, token: IcmpHandlerToken) -> Result<oneshot::Receiver<IcmpReply>> {
        let (tx, rx) = oneshot::channel();
        if self
            .handlers
            .lock()
            .await
            .insert(token.clone(), tx)
            .is_some()
        {
            return Err(anyhow!("duplicate icmp request: {:?}", token));
        }
        Ok(rx)
    }

    async fn remove_handler(&self, token: IcmpHandlerToken) -> Result<()> {
        self.handlers.lock().await.remove(&token);
        Ok(())
    }

    pub async fn ping4(
        &self,
        addr: Ipv4Addr,
        id: u16,
        seq: u16,
        payload: &[u8],
        deadline: Duration,
    ) -> Result<Option<IcmpReply>> {
        let token = IcmpHandlerToken(IpAddr::V4(addr), Some(id), seq);
        let rx = self.add_handler(token.clone()).await?;

        let echo = IcmpEchoHeader { id, seq };
        let mut header = Icmpv4Header::new(Icmpv4Type::EchoRequest(echo));
        header.update_checksum(payload);
        let mut buffer: Vec<u8> = Vec::new();
        header.write(&mut buffer)?;
        buffer.extend_from_slice(payload);

        self.socket
            .send_to(&buffer, SocketAddr::V4(SocketAddrV4::new(addr, 0)))
            .await?;

        let result = timeout(deadline, rx).await;
        self.remove_handler(token).await?;
        let reply = match result {
            Ok(Ok(packet)) => Some(packet),
            Ok(Err(err)) => return Err(anyhow!("failed to wait for icmp packet: {}", err)),
            Err(_) => None,
        };
        Ok(reply)
    }

    pub async fn ping6(
        &self,
        addr: Ipv6Addr,
        id: u16,
        seq: u16,
        payload: &[u8],
        deadline: Duration,
    ) -> Result<Option<IcmpReply>> {
        let token = IcmpHandlerToken(IpAddr::V6(addr), Some(id), seq);
        let rx = self.add_handler(token.clone()).await?;

        let echo = IcmpEchoHeader { id, seq };
        let header = Icmpv6Header::new(Icmpv6Type::EchoRequest(echo));
        let mut buffer: Vec<u8> = Vec::new();
        header.write(&mut buffer)?;
        buffer.extend_from_slice(payload);

        self.socket
            .send_to(&buffer, SocketAddr::V6(SocketAddrV6::new(addr, 0, 0, 0)))
            .await?;

        let result = timeout(deadline, rx).await;
        self.remove_handler(token).await?;
        let reply = match result {
            Ok(Ok(packet)) => Some(packet),
            Ok(Err(err)) => return Err(anyhow!("failed to wait for icmp packet: {}", err)),
            Err(_) => None,
        };
        Ok(reply)
    }
}

impl Drop for IcmpClient {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}
