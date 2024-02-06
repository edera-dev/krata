use std::os::fd::AsRawFd;
use std::panic::UnwindSafe;
use std::str::FromStr;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use std::{panic, thread};

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use futures::TryStreamExt;
use log::{error, info, warn};
use netlink_packet_route::link::LinkAttribute;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::{self, RawSocket};
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, HardwareAddress, IpCidr};
use tokio::time::sleep;

pub struct NetworkBackend {
    pub interface: String,
    pub device: RawSocket,
    pub addresses: Vec<IpCidr>,
}

unsafe impl Send for NetworkBackend {}
impl UnwindSafe for NetworkBackend {}

pub struct NetworkService {
    pub network: String,
}

impl NetworkService {
    pub fn new(network: String) -> Result<NetworkService> {
        Ok(NetworkService { network })
    }
}

impl NetworkBackend {
    pub fn new(iface: &str, cidrs: &[&str]) -> Result<NetworkBackend> {
        let device = RawSocket::new(iface, smoltcp::phy::Medium::Ethernet)?;
        let mut addresses: Vec<IpCidr> = Vec::new();
        for cidr in cidrs {
            let address =
                IpCidr::from_str(cidr).map_err(|_| anyhow!("failed to parse cidr: {}", *cidr))?;
            addresses.push(address);
        }
        Ok(NetworkBackend {
            interface: iface.to_string(),
            device,
            addresses,
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

    pub fn run(mut self) -> Result<()> {
        let result = panic::catch_unwind(move || self.run_maybe_panic());

        if result.is_err() {
            return Err(anyhow!("network backend has terminated"));
        }

        result.unwrap()
    }

    fn run_maybe_panic(&mut self) -> Result<()> {
        let mac = MacAddr6::random();
        let mac = HardwareAddress::Ethernet(EthernetAddress(mac.to_array()));
        let config = Config::new(mac);
        let mut iface = Interface::new(config, &mut self.device, Instant::now());
        iface.update_ip_addrs(|addrs| {
            addrs
                .extend_from_slice(&self.addresses)
                .expect("failed to set ip addresses");
        });

        let mut sockets = SocketSet::new(vec![]);
        let fd = self.device.as_raw_fd();
        loop {
            let timestamp = Instant::now();
            iface.poll(timestamp, &mut self.device, &mut sockets);
            phy::wait(fd, iface.poll_delay(timestamp, &sockets))?;
        }
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
        let mut network = NetworkBackend::new(&interface, &[&self.network])?;
        info!("initializing network backend for interface {}", interface);
        network.init().await?;
        tokio::time::sleep(Duration::from_secs(1)).await;
        info!("spawning network backend for interface {}", interface);
        thread::spawn(move || {
            if let Err(error) = network.run() {
                error!(
                    "failed to run network backend for interface {}: {}",
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
