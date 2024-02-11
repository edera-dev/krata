use crate::chandev::ChannelDevice;
use crate::nat::NatRouter;
use crate::proxynat::ProxyNatHandlerFactory;
use crate::raw_socket::{AsyncRawSocket, RawSocketProtocol};
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
    ipv4: String,
    ipv6: String,
    interface: String,
}

enum NetworkStackSelect<'a> {
    Receive(&'a [u8]),
    Send(Option<Vec<u8>>),
    Reclaim,
}

struct NetworkStack<'a> {
    mtu: usize,
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
            _ = self.router.process_reclaim() => NetworkStackSelect::Reclaim,
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

            NetworkStackSelect::Reclaim => {}
        }

        Ok(())
    }
}

impl NetworkBackend {
    pub fn new(ipv4: &str, ipv6: &str, interface: &str) -> Result<Self> {
        Ok(Self {
            ipv4: ipv4.to_string(),
            ipv6: ipv6.to_string(),
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
        let mut buffer = vec![0u8; stack.mtu];
        loop {
            stack.poll(&mut buffer).await?;
        }
    }

    fn create_network_stack(&self) -> Result<NetworkStack> {
        let proxy = Box::new(ProxyNatHandlerFactory::new());
        let ipv4 = IpCidr::from_str(&self.ipv4)
            .map_err(|_| anyhow!("failed to parse ipv4 cidr: {}", self.ipv4))?;
        let ipv6 = IpCidr::from_str(&self.ipv4)
            .map_err(|_| anyhow!("failed to parse ipv6 cidr: {}", self.ipv6))?;
        let addresses: Vec<IpCidr> = vec![ipv4, ipv6];
        let mut kdev =
            AsyncRawSocket::bound_to_interface(&self.interface, RawSocketProtocol::Ethernet)?;
        let mtu = kdev.mtu_of_interface(&self.interface)?;
        let (tx_sender, tx_receiver) = channel::<Vec<u8>>(4);
        let mut udev = ChannelDevice::new(mtu, tx_sender.clone());
        let mac = MacAddr6::random();
        let mac = smoltcp::wire::EthernetAddress(mac.to_array());
        let nat = NatRouter::new(proxy, mac, addresses.clone(), tx_sender.clone());
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
            mtu,
            tx: tx_receiver,
            kdev,
            udev,
            interface: iface,
            sockets,
            router: nat,
        })
    }
}
