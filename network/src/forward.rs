use std::{
    collections::HashMap,
    net::{SocketAddr, SocketAddrV4},
};

use anyhow::{anyhow, Result};
use bytes::BytesMut;
use log::{debug, error, info, warn};
use smoltcp::{
    iface::{SocketHandle, SocketSet},
    socket::tcp::State,
    wire::{IpAddress, IpEndpoint, Ipv4Address},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener,
    },
    select,
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
};

const PORT_FORWARD_TCP_BUFFER_SIZE: usize = 65535;

#[derive(Debug, Clone)]
pub enum PortForwardProtocol {
    Tcp,
}

#[derive(Debug, Clone)]
pub struct PortForwardSpec {
    pub protocol: PortForwardProtocol,
    pub host: IpEndpoint,
    pub guest: IpEndpoint,
}

impl PortForwardSpec {
    pub fn listen(protocol: PortForwardProtocol, host: u16, guest: u16) -> PortForwardSpec {
        PortForwardSpec {
            protocol,
            host: IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), host),
            guest: IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), guest),
        }
    }
}

#[derive(Debug)]
pub struct PortForwardInternalEvent {
    pub index: usize,
    pub message: PortForwardInternalMessage,
}

#[derive(Debug)]
pub enum PortForwardInternalMessage {
    IncomingConnection { index: usize },
    DataReceived { index: usize, bytes: BytesMut },
    SocketClosed { index: usize },
}

#[derive(Debug)]
pub struct PortForwardExternalEvent {
    pub index: usize,
    pub message: PortForwardExternalMessage,
}

#[derive(Debug)]
pub enum PortForwardExternalMessage {
    SocketOpened,
    SocketClosed,
    DataReceived(BytesMut),
}

pub struct PortForwardOwner {
    pub spec: PortForwardSpec,
    external_sender: Sender<PortForwardExternalEvent>,
    handle_to_stream: HashMap<SocketHandle, usize>,
    stream_to_handle: HashMap<usize, SocketHandle>,
    active_states: HashMap<usize, bool>,
    outgoing: HashMap<usize, Vec<u8>>,
    _task: JoinHandle<()>,
}

impl PortForwardOwner {
    pub async fn new(
        index: usize,
        spec: PortForwardSpec,
        internal_sender: Sender<PortForwardInternalEvent>,
    ) -> Result<PortForwardOwner> {
        let ip = match spec.host.addr {
            IpAddress::Ipv4(addr) => addr,
            _ => return Err(anyhow!("IPv6 host addr not supported")),
        };
        let listener =
            TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(ip.into(), spec.host.port))).await?;
        info!("bound to host port {} for port forwarding", spec.host.port);
        let (external_sender, external_receiver) = channel(100);
        let task = {
            let internal_sender = internal_sender.clone();
            tokio::task::spawn(async move {
                if let Err(error) =
                    PortForwardOwner::process(index, listener, internal_sender, external_receiver)
                        .await
                {
                    error!("failed to handle port forward: {}", error);
                }
            })
        };
        Ok(PortForwardOwner {
            spec,
            external_sender,
            _task: task,
            outgoing: HashMap::new(),
            handle_to_stream: HashMap::new(),
            stream_to_handle: HashMap::new(),
            active_states: HashMap::new(),
        })
    }

    pub fn map_handle(&mut self, handle: &SocketHandle, stream: usize) {
        self.handle_to_stream.insert(*handle, stream);
        self.stream_to_handle.insert(stream, *handle);
        self.active_states.insert(stream, false);
    }

    pub fn lookup_stream(&self, handle: &SocketHandle) -> Option<usize> {
        self.handle_to_stream.get(handle).copied()
    }

    pub fn lookup_handle(&self, stream: &usize) -> Option<SocketHandle> {
        self.stream_to_handle.get(stream).copied()
    }

    pub fn push_outgoing(&mut self, stream: &usize, bytes: BytesMut) {
        let outgoing = match self.outgoing.get_mut(stream) {
            Some(outgoing) => outgoing,
            None => {
                self.outgoing.insert(*stream, Vec::new());
                self.outgoing.get_mut(stream).unwrap()
            }
        };
        outgoing.extend_from_slice(&bytes);
    }

    pub async fn process_sockets<'a>(&mut self, sockets: &mut SocketSet<'a>) -> Result<()> {
        for (handle, stream) in &self.handle_to_stream.clone() {
            let socket = sockets.get_mut::<smoltcp::socket::tcp::Socket>(*handle);
            let is_active_already = *self.active_states.get(stream).unwrap_or(&false);
            if socket.is_active() && !is_active_already {
                self.active_states.insert(*stream, true);
                let event = PortForwardExternalEvent {
                    index: *stream,
                    message: PortForwardExternalMessage::SocketOpened,
                };
                self.external_sender.send(event).await?;
            }

            let ready = match socket.state() {
                State::Closed
                | State::Listen
                | State::Closing
                | State::LastAck
                | State::TimeWait => false,
                State::FinWait1
                | State::SynSent
                | State::CloseWait
                | State::FinWait2
                | State::SynReceived
                | State::Established => true,
            };

            if !ready {
                continue;
            }

            let bytes_to_socket = if socket.can_send() {
                socket.send_capacity() - socket.send_queue()
            } else {
                0
            };

            let bytes_to_stream = if socket.may_recv() {
                if let Ok(data) = socket.peek(PORT_FORWARD_TCP_BUFFER_SIZE) {
                    if data.is_empty() {
                        None
                    } else {
                        Some(data)
                    }
                } else {
                    None
                }
            } else {
                None
            };

            if let Some(bytes) = bytes_to_stream {
                let event = PortForwardExternalEvent {
                    index: *stream,
                    message: PortForwardExternalMessage::DataReceived(bytes.into()),
                };
                self.external_sender.send(event).await?;
            }

            let outgoing = match self.outgoing.get(stream) {
                Some(value) => value,
                None => {
                    self.outgoing.insert(*stream, Vec::new());
                    self.outgoing.get(stream).unwrap()
                }
            };
            if bytes_to_socket > 0 && !outgoing.is_empty() {
                let mut amount = bytes_to_socket;
                if outgoing.len() < amount {
                    amount = outgoing.len();
                }
                let will_send = outgoing[0..amount].to_vec();
                let actually_sent = socket.send_slice(&will_send)?;
                let will_reallocate = outgoing[actually_sent..].to_vec();
                self.outgoing.insert(*stream, will_reallocate);
            }

            if is_active_already && !socket.is_active() {
                let event = PortForwardExternalEvent {
                    index: *stream,
                    message: PortForwardExternalMessage::SocketClosed,
                };
                self.external_sender.send(event).await?;
                sockets.remove(*handle);
                self.active_states.remove(stream);
                self.handle_to_stream.remove(handle);
                self.stream_to_handle.remove(stream);
                self.outgoing.remove(stream);
            }
        }
        Ok(())
    }

    pub async fn process(
        index: usize,
        listener: TcpListener,
        internal_sender: Sender<PortForwardInternalEvent>,
        mut external_receiver: Receiver<PortForwardExternalEvent>,
    ) -> Result<()> {
        let mut next_stream_index: usize = 0;
        let mut pending_reads: HashMap<usize, OwnedReadHalf> = HashMap::new();
        let mut writes: HashMap<usize, OwnedWriteHalf> = HashMap::new();
        loop {
            select! {
                x = listener.accept() => match x {
                    Ok((stream, _)) => {
                        let stream_index = next_stream_index;
                        next_stream_index += 1;
                        let (read, write) = stream.into_split();
                        writes.insert(stream_index, write);
                        pending_reads.insert(stream_index, read);
                        internal_sender.send(PortForwardInternalEvent {
                            index,
                            message: PortForwardInternalMessage::IncomingConnection { index },
                        }).await?;
                    },
                    Err(error) => {
                        return Err(error.into());
                    }
                },
                x = external_receiver.recv() => match x {
                    Some(event) => match event.message {
                        PortForwardExternalMessage::DataReceived(bytes) => {
                            let Some(stream) = writes.get_mut(&event.index) else {
                                continue;
                            };

                            if let Err(error) = stream.write_all(&bytes).await {
                                warn!("failed to write data to TCP stream: {}", error);
                            }

                            let event = PortForwardInternalEvent {
                                index,
                                message: PortForwardInternalMessage::SocketClosed { index: event.index },
                            };
                            if let Err(error) = internal_sender.send(event).await {
                                debug!("failed to send close from TCP stream: {}", error);
                            }
                        },

                        PortForwardExternalMessage::SocketOpened => {
                            let stream_index = event.index;
                            let Some(mut read) = pending_reads.remove(&stream_index) else {
                                continue;
                            };

                            let stream_internal_sender = internal_sender.clone();
                            tokio::task::spawn(async move {
                                let mut buffer = vec![0u8; 2048];
                                loop {
                                    let size = match read.read(&mut buffer).await {
                                        Ok(size) => size,
                                        Err(error) => {
                                            debug!("failed to read from TCP stream: {}", error);
                                            break;
                                        }
                                    };

                                    if size == 0 {
                                        break;
                                    }

                                    let event = PortForwardInternalEvent {
                                        index,
                                        message: PortForwardInternalMessage::DataReceived {
                                            index: stream_index,
                                            bytes: (&buffer[0..size]).into(),
                                        }
                                    };
                                    if let Err(error) = stream_internal_sender.send(event).await {
                                        debug!("failed to send data from TCP stream: {}", error);
                                        break;
                                    }
                                }

                                let event = PortForwardInternalEvent {
                                    index,
                                    message: PortForwardInternalMessage::SocketClosed { index: stream_index },
                                };
                                if let Err(error) = stream_internal_sender.send(event).await {
                                    debug!("failed to send close from TCP stream: {}", error);
                                }
                            });
                        },

                        PortForwardExternalMessage::SocketClosed => {
                            pending_reads.remove(&event.index);
                            let Some(mut write) = writes.remove(&event.index) else {
                                continue;
                            };
                            let _ = write.shutdown().await;
                        }
                    },

                    None => {
                        return Ok(());
                    }
                }
            };
        }
    }
}
