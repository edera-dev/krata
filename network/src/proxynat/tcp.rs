use std::{
    net::{IpAddr, SocketAddr},
    time::Duration,
};

use anyhow::Result;
use async_trait::async_trait;
use bytes::BytesMut;
use etherparse::{EtherType, Ethernet2Header};
use log::{debug, warn};
use smoltcp::{
    iface::{Config, Interface, SocketSet, SocketStorage},
    phy::Medium,
    socket::tcp::{self, SocketBuffer, State},
    time::Instant,
    wire::{HardwareAddress, IpAddress, IpCidr},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
    select,
    sync::mpsc::channel,
};
use tokio::{sync::mpsc::Receiver, sync::mpsc::Sender};

use crate::{
    chandev::ChannelDevice,
    nat::{NatHandler, NatHandlerContext},
};

const TCP_BUFFER_SIZE: usize = 65535;
const TCP_ACCEPT_TIMEOUT_SECS: u64 = 120;
const TCP_DANGLE_TIMEOUT_SECS: u64 = 10;

pub struct ProxyTcpHandler {
    rx_sender: Sender<BytesMut>,
}

#[async_trait]
impl NatHandler for ProxyTcpHandler {
    async fn receive(&self, data: &[u8]) -> Result<bool> {
        if self.rx_sender.is_closed() {
            Ok(false)
        } else {
            self.rx_sender.try_send(data.into())?;
            Ok(true)
        }
    }
}

#[derive(Debug)]
enum ProxyTcpAcceptSelect {
    Internal(BytesMut),
    TxIpPacket(BytesMut),
    TimePassed,
    DoNothing,
    Close,
}

#[derive(Debug)]
enum ProxyTcpDataSelect {
    ExternalRecv(usize),
    ExternalSent(usize),
    InternalRecv(BytesMut),
    TxIpPacket(BytesMut),
    TimePassed,
    DoNothing,
    Close,
}

#[derive(Debug)]
enum ProxyTcpFinishSelect {
    InternalRecv(BytesMut),
    TxIpPacket(BytesMut),
    Close,
}

impl ProxyTcpHandler {
    pub fn new(rx_sender: Sender<BytesMut>) -> Self {
        ProxyTcpHandler { rx_sender }
    }

    pub async fn spawn(
        &mut self,
        context: NatHandlerContext,
        rx_receiver: Receiver<BytesMut>,
    ) -> Result<()> {
        let external_addr = match context.key.external_ip.addr {
            IpAddress::Ipv4(addr) => {
                SocketAddr::new(IpAddr::V4(addr.0.into()), context.key.external_ip.port)
            }
            IpAddress::Ipv6(addr) => {
                SocketAddr::new(IpAddr::V6(addr.0.into()), context.key.external_ip.port)
            }
        };

        let socket = TcpStream::connect(external_addr).await?;
        tokio::spawn(async move {
            if let Err(error) = ProxyTcpHandler::process(context, socket, rx_receiver).await {
                warn!("processing of tcp proxy failed: {}", error);
            }
        });
        Ok(())
    }

    async fn process(
        context: NatHandlerContext,
        mut external_socket: TcpStream,
        mut rx_receiver: Receiver<BytesMut>,
    ) -> Result<()> {
        let (ip_sender, mut ip_receiver) = channel::<BytesMut>(300);
        let mut external_buffer = vec![0u8; TCP_BUFFER_SIZE];

        let mut device = ChannelDevice::new(
            context.mtu - Ethernet2Header::LEN,
            Medium::Ip,
            ip_sender.clone(),
        );
        let config = Config::new(HardwareAddress::Ip);

        let tcp_rx_buffer = SocketBuffer::new(vec![0; TCP_BUFFER_SIZE]);
        let tcp_tx_buffer = SocketBuffer::new(vec![0; TCP_BUFFER_SIZE]);
        let internal_socket = tcp::Socket::new(tcp_rx_buffer, tcp_tx_buffer);
        let mut iface = Interface::new(config, &mut device, Instant::now());

        iface.update_ip_addrs(|addrs| {
            let _ = addrs.push(IpCidr::new(context.key.external_ip.addr, 0));
        });

        let mut sockets = SocketSet::new([SocketStorage::EMPTY]);
        let internal_socket_handle = sockets.add(internal_socket);
        let (mut external_r, mut external_w) = external_socket.split();

        {
            let socket = sockets.get_mut::<tcp::Socket>(internal_socket_handle);
            socket.connect(
                iface.context(),
                context.key.client_ip,
                context.key.external_ip,
            )?;
        }

        iface.poll(Instant::now(), &mut device, &mut sockets);

        let mut sleeper: Option<tokio::time::Sleep> = None;
        loop {
            let socket = sockets.get_mut::<tcp::Socket>(internal_socket_handle);
            if socket.is_active() && socket.state() != State::SynSent {
                break;
            }

            if socket.state() == State::Closed {
                break;
            }

            let deadline = tokio::time::sleep(Duration::from_secs(TCP_ACCEPT_TIMEOUT_SECS));
            let selection = if let Some(sleep) = sleeper.take() {
                select! {
                    biased;
                    x = rx_receiver.recv() => if let Some(data) = x {
                        ProxyTcpAcceptSelect::Internal(data)
                    } else {
                        ProxyTcpAcceptSelect::Close
                    },
                    x = ip_receiver.recv() => if let Some(data) = x {
                        ProxyTcpAcceptSelect::TxIpPacket(data)
                    } else {
                        ProxyTcpAcceptSelect::Close
                    },
                    _ = sleep => ProxyTcpAcceptSelect::TimePassed,
                    _ = deadline => ProxyTcpAcceptSelect::Close,
                }
            } else {
                select! {
                    biased;
                    x = rx_receiver.recv() => if let Some(data) = x {
                        ProxyTcpAcceptSelect::Internal(data)
                    } else {
                        ProxyTcpAcceptSelect::Close
                    },
                    x = ip_receiver.recv() => if let Some(data) = x {
                        ProxyTcpAcceptSelect::TxIpPacket(data)
                    } else {
                        ProxyTcpAcceptSelect::Close
                    },
                    _ = std::future::ready(()) => ProxyTcpAcceptSelect::DoNothing,
                    _ = deadline => ProxyTcpAcceptSelect::Close,
                }
            };
            match selection {
                ProxyTcpAcceptSelect::TimePassed => {
                    iface.poll(Instant::now(), &mut device, &mut sockets);
                }

                ProxyTcpAcceptSelect::DoNothing => {
                    sleeper = Some(tokio::time::sleep(Duration::from_millis(50)));
                }

                ProxyTcpAcceptSelect::Internal(data) => {
                    let (_, payload) = Ethernet2Header::from_slice(&data)?;
                    device.rx = Some(payload.into());
                    iface.poll(Instant::now(), &mut device, &mut sockets);
                }

                ProxyTcpAcceptSelect::TxIpPacket(payload) => {
                    let mut buffer: Vec<u8> = Vec::new();
                    let header = Ethernet2Header {
                        source: context.key.local_mac.0,
                        destination: context.key.client_mac.0,
                        ether_type: match context.key.external_ip.addr {
                            IpAddress::Ipv4(_) => EtherType::IPV4,
                            IpAddress::Ipv6(_) => EtherType::IPV6,
                        },
                    };
                    header.write(&mut buffer)?;
                    buffer.extend_from_slice(&payload);
                    if let Err(error) = context.try_send(buffer.as_slice().into()) {
                        debug!("failed to transmit tcp packet: {}", error);
                    }
                }

                ProxyTcpAcceptSelect::Close => {
                    break;
                }
            }
        }

        let accepted = if sockets
            .get_mut::<tcp::Socket>(internal_socket_handle)
            .is_active()
        {
            true
        } else {
            debug!("failed to accept tcp connection from client");
            false
        };

        let mut already_shutdown = false;
        let mut sleeper: Option<tokio::time::Sleep> = None;
        loop {
            if !accepted {
                break;
            }

            let socket = sockets.get_mut::<tcp::Socket>(internal_socket_handle);

            match socket.state() {
                State::Closed
                | State::Listen
                | State::Closing
                | State::LastAck
                | State::TimeWait => {
                    break;
                }
                State::FinWait1
                | State::SynSent
                | State::CloseWait
                | State::FinWait2
                | State::SynReceived
                | State::Established => {}
            }

            let bytes_to_client = if socket.can_send() {
                socket.send_capacity() - socket.send_queue()
            } else {
                0
            };

            let (bytes_to_external, do_shutdown) = if socket.may_recv() {
                if let Ok(data) = socket.peek(TCP_BUFFER_SIZE) {
                    if data.is_empty() {
                        (None, false)
                    } else {
                        (Some(data), false)
                    }
                } else {
                    (None, false)
                }
            } else if !already_shutdown && matches!(socket.state(), State::CloseWait) {
                (None, true)
            } else {
                (None, false)
            };
            let selection = if let Some(sleep) = sleeper.take() {
                if !do_shutdown {
                    select! {
                        biased;
                        x = ip_receiver.recv() => if let Some(data) = x {
                            ProxyTcpDataSelect::TxIpPacket(data)
                        } else {
                            ProxyTcpDataSelect::Close
                        },
                        x = rx_receiver.recv() => if let Some(data) = x {
                            ProxyTcpDataSelect::InternalRecv(data)
                        } else {
                            ProxyTcpDataSelect::Close
                        },
                        x = external_w.write(bytes_to_external.unwrap_or(b"")), if bytes_to_external.is_some() => ProxyTcpDataSelect::ExternalSent(x?),
                        x = external_r.read(&mut external_buffer[..bytes_to_client]), if bytes_to_client > 0 => ProxyTcpDataSelect::ExternalRecv(x?),
                        _ = sleep => ProxyTcpDataSelect::TimePassed,
                    }
                } else {
                    select! {
                        biased;
                        x = ip_receiver.recv() => if let Some(data) = x {
                            ProxyTcpDataSelect::TxIpPacket(data)
                        } else {
                            ProxyTcpDataSelect::Close
                        },
                        x = rx_receiver.recv() => if let Some(data) = x {
                            ProxyTcpDataSelect::InternalRecv(data)
                        } else {
                            ProxyTcpDataSelect::Close
                        },
                        _ = external_w.shutdown() => ProxyTcpDataSelect::ExternalSent(0),
                        x = external_r.read(&mut external_buffer[..bytes_to_client]), if bytes_to_client > 0 => ProxyTcpDataSelect::ExternalRecv(x?),
                        _ = sleep => ProxyTcpDataSelect::TimePassed,
                    }
                }
            } else if !do_shutdown {
                select! {
                    biased;
                    x = ip_receiver.recv() => if let Some(data) = x {
                        ProxyTcpDataSelect::TxIpPacket(data)
                    } else {
                        ProxyTcpDataSelect::Close
                    },
                    x = rx_receiver.recv() => if let Some(data) = x {
                        ProxyTcpDataSelect::InternalRecv(data)
                    } else {
                        ProxyTcpDataSelect::Close
                    },
                    x = external_w.write(bytes_to_external.unwrap_or(b"")), if bytes_to_external.is_some() => ProxyTcpDataSelect::ExternalSent(x?),
                    x = external_r.read(&mut external_buffer[..bytes_to_client]), if bytes_to_client > 0 => ProxyTcpDataSelect::ExternalRecv(x?),
                    _ = std::future::ready(()) => ProxyTcpDataSelect::DoNothing,
                }
            } else {
                select! {
                    biased;
                    x = ip_receiver.recv() => if let Some(data) = x {
                        ProxyTcpDataSelect::TxIpPacket(data)
                    } else {
                        ProxyTcpDataSelect::Close
                    },
                    x = rx_receiver.recv() => if let Some(data) = x {
                        ProxyTcpDataSelect::InternalRecv(data)
                    } else {
                        ProxyTcpDataSelect::Close
                    },
                    _ = external_w.shutdown() => ProxyTcpDataSelect::ExternalSent(0),
                    x = external_r.read(&mut external_buffer[..bytes_to_client]), if bytes_to_client > 0 => ProxyTcpDataSelect::ExternalRecv(x?),
                    _ = std::future::ready(()) => ProxyTcpDataSelect::DoNothing,
                }
            };
            match selection {
                ProxyTcpDataSelect::ExternalRecv(size) => {
                    if size == 0 {
                        socket.close();
                    } else {
                        socket.send_slice(&external_buffer[..size])?;
                    }
                }

                ProxyTcpDataSelect::ExternalSent(size) => {
                    if size == 0 {
                        already_shutdown = true;
                    } else {
                        socket.recv(|_| (size, ()))?;
                    }
                }

                ProxyTcpDataSelect::InternalRecv(data) => {
                    let (_, payload) = Ethernet2Header::from_slice(&data)?;
                    device.rx = Some(payload.into());
                    iface.poll(Instant::now(), &mut device, &mut sockets);
                }

                ProxyTcpDataSelect::TxIpPacket(payload) => {
                    let mut buffer: Vec<u8> = Vec::new();
                    let header = Ethernet2Header {
                        source: context.key.local_mac.0,
                        destination: context.key.client_mac.0,
                        ether_type: match context.key.external_ip.addr {
                            IpAddress::Ipv4(_) => EtherType::IPV4,
                            IpAddress::Ipv6(_) => EtherType::IPV6,
                        },
                    };
                    header.write(&mut buffer)?;
                    buffer.extend_from_slice(&payload);
                    if let Err(error) = context.try_send(buffer.as_slice().into()) {
                        debug!("failed to transmit tcp packet: {}", error);
                    }
                }

                ProxyTcpDataSelect::TimePassed => {
                    iface.poll(Instant::now(), &mut device, &mut sockets);
                }

                ProxyTcpDataSelect::DoNothing => {
                    sleeper = Some(tokio::time::sleep(Duration::from_millis(50)));
                }

                ProxyTcpDataSelect::Close => {
                    break;
                }
            }
        }

        let _ = external_socket.shutdown().await;
        drop(external_socket);

        loop {
            let deadline = tokio::time::sleep(Duration::from_secs(TCP_DANGLE_TIMEOUT_SECS));
            tokio::pin!(deadline);

            let selection = select! {
                biased;
                x = ip_receiver.recv() => if let Some(data) = x {
                    ProxyTcpFinishSelect::TxIpPacket(data)
                } else {
                    ProxyTcpFinishSelect::Close
                },
                x = rx_receiver.recv() => if let Some(data) = x {
                    ProxyTcpFinishSelect::InternalRecv(data)
                } else {
                    ProxyTcpFinishSelect::Close
                },
                _ = deadline => ProxyTcpFinishSelect::Close,
            };

            match selection {
                ProxyTcpFinishSelect::InternalRecv(data) => {
                    let (_, payload) = Ethernet2Header::from_slice(&data)?;
                    device.rx = Some(payload.into());
                    iface.poll(Instant::now(), &mut device, &mut sockets);
                }

                ProxyTcpFinishSelect::TxIpPacket(payload) => {
                    let mut buffer: Vec<u8> = Vec::new();
                    let header = Ethernet2Header {
                        source: context.key.local_mac.0,
                        destination: context.key.client_mac.0,
                        ether_type: match context.key.external_ip.addr {
                            IpAddress::Ipv4(_) => EtherType::IPV4,
                            IpAddress::Ipv6(_) => EtherType::IPV6,
                        },
                    };
                    header.write(&mut buffer)?;
                    buffer.extend_from_slice(&payload);
                    if let Err(error) = context.try_send(buffer.as_slice().into()) {
                        debug!("failed to transmit tcp packet: {}", error);
                    }
                }

                ProxyTcpFinishSelect::Close => {
                    break;
                }
            }
        }

        context.reclaim().await?;

        Ok(())
    }
}
