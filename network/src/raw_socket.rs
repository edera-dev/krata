use anyhow::{anyhow, Result};
use bytes::BytesMut;
use log::{debug, warn};
use std::io::ErrorKind;
use std::os::fd::{FromRawFd, IntoRawFd};
use std::os::unix::io::{AsRawFd, RawFd};
use std::sync::Arc;
use std::{io, mem};
use tokio::net::UdpSocket;
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver, Sender};
use tokio::task::JoinHandle;

const RAW_SOCKET_TRANSMIT_QUEUE_LEN: usize = 3000;
const RAW_SOCKET_RECEIVE_QUEUE_LEN: usize = 3000;

#[derive(Debug)]
pub enum RawSocketProtocol {
    Icmpv4,
    Icmpv6,
    Ethernet,
}

impl RawSocketProtocol {
    pub fn to_socket_domain(&self) -> i32 {
        match self {
            RawSocketProtocol::Icmpv4 => libc::AF_INET,
            RawSocketProtocol::Icmpv6 => libc::AF_INET6,
            RawSocketProtocol::Ethernet => libc::AF_PACKET,
        }
    }

    pub fn to_socket_protocol(&self) -> u16 {
        match self {
            RawSocketProtocol::Icmpv4 => libc::IPPROTO_ICMP as u16,
            RawSocketProtocol::Icmpv6 => libc::IPPROTO_ICMPV6 as u16,
            RawSocketProtocol::Ethernet => (libc::ETH_P_ALL as u16).to_be(),
        }
    }

    pub fn to_socket_type(&self) -> i32 {
        libc::SOCK_RAW
    }
}

const SIOCGIFINDEX: libc::c_ulong = 0x8933;
const SIOCGIFMTU: libc::c_ulong = 0x8921;

#[derive(Debug)]
pub struct RawSocketHandle {
    protocol: RawSocketProtocol,
    lower: libc::c_int,
}

impl AsRawFd for RawSocketHandle {
    fn as_raw_fd(&self) -> RawFd {
        self.lower
    }
}

impl IntoRawFd for RawSocketHandle {
    fn into_raw_fd(self) -> RawFd {
        let fd = self.lower;
        mem::forget(self);
        fd
    }
}

impl RawSocketHandle {
    pub fn new(protocol: RawSocketProtocol) -> io::Result<RawSocketHandle> {
        let lower = unsafe {
            let lower = libc::socket(
                protocol.to_socket_domain(),
                protocol.to_socket_type() | libc::SOCK_NONBLOCK,
                protocol.to_socket_protocol() as i32,
            );
            if lower == -1 {
                return Err(io::Error::last_os_error());
            }
            lower
        };

        Ok(RawSocketHandle { protocol, lower })
    }

    pub fn bound_to_interface(interface: &str, protocol: RawSocketProtocol) -> Result<Self> {
        let mut socket = RawSocketHandle::new(protocol)?;
        socket.bind_to_interface(interface)?;
        Ok(socket)
    }

    pub fn bind_to_interface(&mut self, interface: &str) -> io::Result<()> {
        let mut ifreq = ifreq_for(interface);
        let sockaddr = libc::sockaddr_ll {
            sll_family: libc::AF_PACKET as u16,
            sll_protocol: self.protocol.to_socket_protocol(),
            sll_ifindex: ifreq_ioctl(self.lower, &mut ifreq, SIOCGIFINDEX)?,
            sll_hatype: 1,
            sll_pkttype: 0,
            sll_halen: 6,
            sll_addr: [0; 8],
        };

        unsafe {
            let res = libc::bind(
                self.lower,
                &sockaddr as *const libc::sockaddr_ll as *const libc::sockaddr,
                mem::size_of::<libc::sockaddr_ll>() as libc::socklen_t,
            );
            if res == -1 {
                return Err(io::Error::last_os_error());
            }
        }

        Ok(())
    }

    pub fn mtu_of_interface(&mut self, interface: &str) -> io::Result<usize> {
        let mut ifreq = ifreq_for(interface);
        ifreq_ioctl(self.lower, &mut ifreq, SIOCGIFMTU).map(|mtu| mtu as usize)
    }

    pub fn recv(&self, buffer: &mut [u8]) -> io::Result<usize> {
        unsafe {
            let len = libc::recv(
                self.lower,
                buffer.as_mut_ptr() as *mut libc::c_void,
                buffer.len(),
                0,
            );
            if len == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(len as usize)
        }
    }

    pub fn send(&self, buffer: &[u8]) -> io::Result<usize> {
        unsafe {
            let len = libc::send(
                self.lower,
                buffer.as_ptr() as *const libc::c_void,
                buffer.len(),
                0,
            );
            if len == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(len as usize)
        }
    }
}

impl Drop for RawSocketHandle {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.lower);
        }
    }
}

#[repr(C)]
#[derive(Debug)]
struct Ifreq {
    ifr_name: [libc::c_char; libc::IF_NAMESIZE],
    ifr_data: libc::c_int,
}

fn ifreq_for(name: &str) -> Ifreq {
    let mut ifreq = Ifreq {
        ifr_name: [0; libc::IF_NAMESIZE],
        ifr_data: 0,
    };
    for (i, byte) in name.as_bytes().iter().enumerate() {
        ifreq.ifr_name[i] = *byte as libc::c_char
    }
    ifreq
}

fn ifreq_ioctl(
    lower: libc::c_int,
    ifreq: &mut Ifreq,
    cmd: libc::c_ulong,
) -> io::Result<libc::c_int> {
    unsafe {
        let res = libc::ioctl(lower, cmd as _, ifreq as *mut Ifreq);
        if res == -1 {
            return Err(io::Error::last_os_error());
        }
    }

    Ok(ifreq.ifr_data)
}

pub struct AsyncRawSocketChannel {
    pub sender: Sender<BytesMut>,
    pub receiver: Receiver<BytesMut>,
    _task: Arc<JoinHandle<()>>,
}

enum AsyncRawSocketChannelSelect {
    TransmitPacket(Option<BytesMut>),
    Readable(()),
}

impl AsyncRawSocketChannel {
    pub fn new(mtu: usize, socket: RawSocketHandle) -> Result<AsyncRawSocketChannel> {
        let (transmit_sender, transmit_receiver) = channel(RAW_SOCKET_TRANSMIT_QUEUE_LEN);
        let (receive_sender, receive_receiver) = channel(RAW_SOCKET_RECEIVE_QUEUE_LEN);
        let task = AsyncRawSocketChannel::launch(mtu, socket, transmit_receiver, receive_sender)?;
        Ok(AsyncRawSocketChannel {
            sender: transmit_sender,
            receiver: receive_receiver,
            _task: Arc::new(task),
        })
    }

    fn launch(
        mtu: usize,
        socket: RawSocketHandle,
        transmit_receiver: Receiver<BytesMut>,
        receive_sender: Sender<BytesMut>,
    ) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            if let Err(error) =
                AsyncRawSocketChannel::process(mtu, socket, transmit_receiver, receive_sender).await
            {
                warn!("failed to process raw socket: {}", error);
            }
        }))
    }

    async fn process(
        mtu: usize,
        socket: RawSocketHandle,
        mut transmit_receiver: Receiver<BytesMut>,
        receive_sender: Sender<BytesMut>,
    ) -> Result<()> {
        let socket = unsafe { std::net::UdpSocket::from_raw_fd(socket.into_raw_fd()) };
        let socket = UdpSocket::from_std(socket)?;

        let mut buffer = vec![0; mtu];
        loop {
            let selection = select! {
                x = transmit_receiver.recv() => AsyncRawSocketChannelSelect::TransmitPacket(x),
                x = socket.readable() => AsyncRawSocketChannelSelect::Readable(x?),
            };

            match selection {
                AsyncRawSocketChannelSelect::Readable(_) => {
                    match socket.try_recv(&mut buffer) {
                        Ok(len) => {
                            if len == 0 {
                                continue;
                            }
                            let buffer = (&buffer[0..len]).into();
                            if let Err(error) = receive_sender.try_send(buffer) {
                                debug!("raw socket failed to process received packet: {}", error);
                            }
                        }

                        Err(ref error) => {
                            if error.kind() == ErrorKind::WouldBlock {
                                continue;
                            }
                            return Err(anyhow!("failed to read from raw socket: {}", error));
                        }
                    };
                }

                AsyncRawSocketChannelSelect::TransmitPacket(Some(packet)) => {
                    match socket.try_send(&packet) {
                        Ok(_len) => {}
                        Err(ref error) => {
                            if error.kind() == ErrorKind::WouldBlock {
                                debug!("failed to transmit: would block");
                                continue;
                            }
                            return Err(anyhow!("failed to write to raw socket: {}", error));
                        }
                    };
                }

                AsyncRawSocketChannelSelect::TransmitPacket(None) => {
                    break;
                }
            }
        }

        Ok(())
    }
}
