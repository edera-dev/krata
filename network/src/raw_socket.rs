use anyhow::Result;
use futures::ready;
use std::os::fd::IntoRawFd;
use std::os::unix::io::{AsRawFd, RawFd};
use std::pin::Pin;
use std::task::{Context, Poll};
use std::{io, mem};
use tokio::io::unix::AsyncFd;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

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

pub struct AsyncRawSocket {
    inner: AsyncFd<RawSocketHandle>,
}

impl AsyncRawSocket {
    pub fn new(socket: RawSocketHandle) -> Result<Self> {
        Ok(Self {
            inner: AsyncFd::new(socket)?,
        })
    }

    pub fn bound_to_interface(interface: &str, protocol: RawSocketProtocol) -> Result<Self> {
        let socket = RawSocketHandle::bound_to_interface(interface, protocol)?;
        AsyncRawSocket::new(socket)
    }

    pub fn mtu_of_interface(&mut self, interface: &str) -> Result<usize> {
        Ok(self.inner.get_mut().mtu_of_interface(interface)?)
    }
}

impl TryFrom<RawSocketHandle> for AsyncRawSocket {
    type Error = anyhow::Error;

    fn try_from(value: RawSocketHandle) -> Result<Self, Self::Error> {
        Ok(Self {
            inner: AsyncFd::new(value)?,
        })
    }
}

impl AsyncRead for AsyncRawSocket {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        loop {
            let mut guard = ready!(self.inner.poll_read_ready(cx))?;

            let unfilled = buf.initialize_unfilled();
            match guard.try_io(|inner| inner.get_ref().recv(unfilled)) {
                Ok(Ok(len)) => {
                    buf.advance(len);
                    return Poll::Ready(Ok(()));
                }
                Ok(Err(err)) => return Poll::Ready(Err(err)),
                Err(_would_block) => continue,
            }
        }
    }
}

impl AsyncWrite for AsyncRawSocket {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        loop {
            let mut guard = ready!(self.inner.poll_write_ready(cx))?;

            match guard.try_io(|inner| inner.get_ref().send(buf)) {
                Ok(result) => return Poll::Ready(result),
                Err(_would_block) => continue,
            }
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }

    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}
