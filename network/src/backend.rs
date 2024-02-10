use crate::chandev::ChannelDevice;
use crate::nat::NatRouter;
use crate::proxynat::ProxyNatHandlerFactory;
use crate::raw_socket::AsyncRawSocket;
use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use futures::TryStreamExt;
use log::warn;
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpCidr};
use std::str::FromStr;
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver};

#[derive(Clone)]
pub struct NetworkBackend {
    network: String,
    interface: String,
}

enum NetworkStackSelect<'a> {
    Receive(&'a [u8]),
    Send(Option<Vec<u8>>),
}

struct NetworkStack<'a> {
    tx: Receiver<Vec<u8>>,
    kdev: AsyncRawSocket,
    udev: ChannelDevice,
    interface: Interface,
    sockets: SocketSet<'a>,
    router: NatRouter,
}

impl NetworkStack<'_> {
    async fn poll(&mut self, receive_buffer: &mut [u8]) -> Result<()> {
        let what = select! {
            x = self.tx.recv() => NetworkStackSelect::Send(x),
            x = self.kdev.read(receive_buffer) => NetworkStackSelect::Receive(&receive_buffer[0..x?]),
        };

        match what {
            NetworkStackSelect::Send(packet) => {
                if let Some(packet) = packet {
                    self.kdev.write_all(&packet).await?
                }
            }

            NetworkStackSelect::Receive(packet) => {
                if let Err(error) = self.router.process(packet).await {
                    warn!("router failed to process packet: {}", error);
                }

                self.udev.rx = Some(packet.to_vec());
                let timestamp = Instant::now();
                self.interface
                    .poll(timestamp, &mut self.udev, &mut self.sockets);
            }
        }

        Ok(())
    }
}

impl NetworkBackend {
    pub fn new(network: &str, interface: &str) -> Result<Self> {
        Ok(Self {
            network: network.to_string(),
            interface: interface.to_string(),
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

    pub async fn run(&self) -> Result<()> {
        let mut stack = self.create_network_stack()?;
        let mut buffer = vec![0u8; 1500];
        loop {
            stack.poll(&mut buffer).await?;
        }
    }

    fn create_network_stack(&self) -> Result<NetworkStack> {
        let proxy = Box::new(ProxyNatHandlerFactory::new());
        let address = IpCidr::from_str(&self.network)
            .map_err(|_| anyhow!("failed to parse cidr: {}", self.network))?;
        let addresses: Vec<IpCidr> = vec![address];
        let kdev = AsyncRawSocket::bind(&self.interface)?;
        let (tx_sender, tx_receiver) = channel::<Vec<u8>>(4);
        let mut udev = ChannelDevice::new(1500, tx_sender.clone());
        let mac = MacAddr6::random();
        let mac = smoltcp::wire::EthernetAddress(mac.to_array());
        let nat = NatRouter::new(proxy, mac, tx_sender.clone());
        let mac = HardwareAddress::Ethernet(mac);
        let config = Config::new(mac);
        let mut iface = Interface::new(config, &mut udev, Instant::now());
        iface.update_ip_addrs(|addrs| {
            addrs
                .extend_from_slice(&addresses)
                .expect("failed to set ip addresses");
        });
        let sockets = SocketSet::new(vec![]);
        Ok(NetworkStack {
            tx: tx_receiver,
            kdev,
            udev,
            interface: iface,
            sockets,
            router: nat,
        })
    }
}
