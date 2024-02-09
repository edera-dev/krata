use crate::raw_socket::{AsyncRawSocket, RawSocket};
use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use futures::channel::oneshot;
use futures::{try_join, TryStreamExt};
use ipstack::stream::IpStackStream;
use log::{debug, warn};
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpCidr};
use std::os::fd::AsRawFd;
use std::str::FromStr;
use std::thread;
use std::time::Duration;
use tokio::net::TcpStream;
use udp_stream::UdpStream;

pub trait NetworkSlice {
    async fn run(&self) -> Result<()>;
}

pub struct NetworkBackend {
    pub interface: String,
    local: LocalNetworkSlice,
    internet: InternetNetworkSlice,
}

impl NetworkBackend {
    pub fn new(network: &str, interface: &str) -> Result<Self> {
        Ok(Self {
            interface: interface.to_string(),
            local: LocalNetworkSlice::new(network, interface)?,
            internet: InternetNetworkSlice::new(interface)?,
        })
    }

    pub async fn init(&mut self) -> Result<()> {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle
            .link()
            .get()
            .match_name(self.interface.to_string())
            .execute();
        let link = links.try_next().await?;
        if link.is_none() {
            return Err(anyhow!(
                "unable to find network interface named {}",
                self.interface
            ));
        }
        let link = link.unwrap();
        handle.link().set(link.header.index).up().execute().await?;
        tokio::time::sleep(Duration::from_secs(3)).await;
        Ok(())
    }

    pub async fn run(&mut self) -> Result<()> {
        try_join!(self.local.run(), self.internet.run()).map(|_| ())
    }
}

#[derive(Clone)]
struct LocalNetworkSlice {
    network: String,
    interface: String,
}

impl LocalNetworkSlice {
    fn new(network: &str, interface: &str) -> Result<Self> {
        Ok(Self {
            network: network.to_string(),
            interface: interface.to_string(),
        })
    }

    fn run_blocking(&self) -> Result<()> {
        let address = IpCidr::from_str(&self.network)
            .map_err(|_| anyhow!("failed to parse cidr: {}", self.network))?;
        let addresses: Vec<IpCidr> = vec![address];
        let mut socket = RawSocket::new(&self.interface)?;
        let mac = MacAddr6::random();
        let mac = HardwareAddress::Ethernet(smoltcp::wire::EthernetAddress(mac.to_array()));
        let config = Config::new(mac);
        let mut iface = Interface::new(config, &mut socket, Instant::now());
        iface.update_ip_addrs(|addrs| {
            addrs
                .extend_from_slice(&addresses)
                .expect("failed to set ip addresses");
        });

        let mut sockets = SocketSet::new(vec![]);
        let fd = socket.as_raw_fd();
        loop {
            let timestamp = Instant::now();
            iface.poll(timestamp, &mut socket, &mut sockets);
            smoltcp::phy::wait(fd, iface.poll_delay(timestamp, &sockets))?;
        }
    }
}

impl NetworkSlice for LocalNetworkSlice {
    async fn run(&self) -> Result<()> {
        let (tx, rx) = oneshot::channel();
        let me = self.clone();
        thread::spawn(move || {
            let _ = tx.send(me.run_blocking());
        });
        rx.await?
    }
}

struct InternetNetworkSlice {
    interface: String,
}

impl InternetNetworkSlice {
    pub fn new(interface: &str) -> Result<Self> {
        Ok(Self {
            interface: interface.to_string(),
        })
    }

    async fn process_stream(&self, stream: IpStackStream) -> Result<()> {
        match stream {
            IpStackStream::Tcp(mut tcp) => {
                debug!("tcp: {}", tcp.peer_addr());
                tokio::spawn(async move {
                    if let Ok(mut stream) = TcpStream::connect(tcp.peer_addr()).await {
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut tcp).await;
                    } else {
                        warn!("failed to connect to tcp address: {}", tcp.peer_addr());
                    }
                });
            }

            IpStackStream::Udp(mut udp) => {
                debug!("udp: {}", udp.peer_addr());
                tokio::spawn(async move {
                    if let Ok(mut stream) = UdpStream::connect(udp.peer_addr()).await {
                        let _ = tokio::io::copy_bidirectional(&mut stream, &mut udp).await;
                    } else {
                        warn!("failed to connect to udp address: {}", udp.peer_addr());
                    }
                });
            }

            IpStackStream::UnknownTransport(u) => {
                debug!("unknown transport: {}", u.dst_addr());
            }

            IpStackStream::UnknownNetwork(packet) => {
                debug!("unknown network: {:?}", packet);
            }
        }
        Ok(())
    }
}

impl NetworkSlice for InternetNetworkSlice {
    async fn run(&self) -> Result<()> {
        let mut config = ipstack::IpStackConfig::default();
        config.mtu(1500);
        config.tcp_timeout(std::time::Duration::from_secs(600));
        config.udp_timeout(std::time::Duration::from_secs(10));

        let socket = AsyncRawSocket::bind(&self.interface)?;
        let mut stack = ipstack::IpStack::new(config, socket);

        while let Ok(stream) = stack.accept().await {
            self.process_stream(stream).await?
        }
        Ok(())
    }
}
