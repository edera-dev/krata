use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::{anyhow, Result};
use futures::TryStreamExt;
use ipstack::stream::IpStackStream;
use log::{debug, error, info, warn};
use netlink_packet_route::link::LinkAttribute;
use raw_socket::{AsyncRawSocket, RawSocket};
use tokio::net::TcpStream;
use tokio::time::sleep;
use udp_stream::UdpStream;

mod raw_socket;

pub struct NetworkBackend {
    pub interface: String,
}
pub struct NetworkService {
    pub network: String,
}

impl NetworkService {
    pub fn new(network: String) -> Result<NetworkService> {
        Ok(NetworkService { network })
    }
}

impl NetworkBackend {
    pub fn new(iface: &str) -> Result<NetworkBackend> {
        Ok(NetworkBackend {
            interface: iface.to_string(),
        })
    }

    pub async fn init(&mut self) -> Result<()> {
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle
            .link()
            .get()
            .match_name(self.interface.clone())
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
        let mut config = ipstack::IpStackConfig::default();
        config.mtu(1500);
        config.tcp_timeout(std::time::Duration::from_secs(600)); // 10 minutes
        config.udp_timeout(std::time::Duration::from_secs(10)); // 10 seconds

        let mut socket = RawSocket::new(&self.interface)?;
        socket.bind_interface()?;
        let socket = AsyncRawSocket::new(socket)?;
        let mut stack = ipstack::IpStack::new(config, socket);

        while let Ok(stream) = stack.accept().await {
            self.process_stream(stream).await?
        }
        Ok(())
    }

    async fn process_stream(&mut self, stream: IpStackStream) -> Result<()> {
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

impl NetworkService {
    pub async fn watch(&mut self) -> Result<()> {
        let spawned: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);
        loop {
            let mut stream = handle.link().get().execute();
            while let Some(message) = stream.try_next().await? {
                let mut name: Option<String> = None;
                for attribute in &message.attributes {
                    if let LinkAttribute::IfName(if_name) = attribute {
                        name = Some(if_name.clone());
                    }
                }

                if name.is_none() {
                    continue;
                }

                let name = name.unwrap();
                if !name.starts_with("vif") {
                    continue;
                }

                if let Ok(spawns) = spawned.lock() {
                    if spawns.contains(&name) {
                        continue;
                    }
                }

                if let Err(error) = self.add_network_backend(&name, spawned.clone()).await {
                    warn!(
                        "failed to initialize network backend for interface {}: {}",
                        name, error
                    );
                }

                if let Ok(mut spawns) = spawned.lock() {
                    spawns.push(name.clone());
                }
            }

            sleep(Duration::from_secs(2)).await;
        }
    }

    async fn add_network_backend(
        &mut self,
        interface: &str,
        spawned: Arc<Mutex<Vec<String>>>,
    ) -> Result<()> {
        let interface = interface.to_string();
        let mut network = NetworkBackend::new(&interface)?;
        info!("initializing network backend for interface {}", interface);
        network.init().await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        info!("spawning network backend for interface {}", interface);
        tokio::spawn(async move {
            if let Err(error) = network.run().await {
                error!(
                    "network backend for interface {} has been stopped: {}",
                    interface, error
                );
            }

            if let Ok(mut spawns) = spawned.lock() {
                if let Some(position) = spawns.iter().position(|x| *x == interface) {
                    spawns.remove(position);
                }
            }
        });
        Ok(())
    }
}
