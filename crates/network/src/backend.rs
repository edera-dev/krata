use crate::autonet::NetworkMetadata;
use crate::chandev::ChannelDevice;
use crate::nat::Nat;
use crate::proxynat::ProxyNatHandlerFactory;
use crate::raw_socket::{AsyncRawSocketChannel, RawSocketHandle, RawSocketProtocol};
use crate::vbridge::{BridgeJoinHandle, VirtualBridge};
use crate::EXTRA_MTU;
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use futures::TryStreamExt;
use log::{info, trace, warn};
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::Medium;
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpCidr};
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver};
use tokio::task::JoinHandle;

const TX_CHANNEL_BUFFER_LEN: usize = 3000;

#[derive(Clone)]
pub struct NetworkBackend {
    metadata: NetworkMetadata,
    bridge: VirtualBridge,
}

#[derive(Debug)]
enum NetworkStackSelect {
    Receive(Option<BytesMut>),
    Send(Option<BytesMut>),
}

struct NetworkStack<'a> {
    tx: Receiver<BytesMut>,
    kdev: AsyncRawSocketChannel,
    udev: ChannelDevice,
    interface: Interface,
    sockets: SocketSet<'a>,
    nat: Nat,
    bridge: BridgeJoinHandle,
}

impl NetworkStack<'_> {
    async fn poll(&mut self) -> Result<bool> {
        let what = select! {
            biased;
            x = self.kdev.receiver.recv() => NetworkStackSelect::Receive(x),
            x = self.tx.recv() => NetworkStackSelect::Send(x),
            x = self.bridge.from_bridge_receiver.recv() => NetworkStackSelect::Send(x),
            x = self.bridge.from_broadcast_receiver.recv() => NetworkStackSelect::Send(x.ok()),
        };

        match what {
            NetworkStackSelect::Receive(Some(packet)) => {
                if let Err(error) = self.bridge.to_bridge_sender.try_send(packet.clone()) {
                    trace!("failed to send zone packet to bridge: {}", error);
                }

                if let Err(error) = self.nat.receive_sender.try_send(packet.clone()) {
                    trace!("failed to send zone packet to nat: {}", error);
                }

                self.udev.rx = Some(packet);
                self.interface
                    .poll(Instant::now(), &mut self.udev, &mut self.sockets);
            }

            NetworkStackSelect::Send(Some(packet)) => {
                if let Err(error) = self.kdev.sender.try_send(packet) {
                    warn!("failed to transmit packet to interface: {}", error);
                }
            }

            NetworkStackSelect::Receive(None) | NetworkStackSelect::Send(None) => {
                return Ok(false);
            }
        }

        Ok(true)
    }
}

impl NetworkBackend {
    pub fn new(metadata: NetworkMetadata, bridge: VirtualBridge) -> Result<Self> {
        Ok(Self { metadata, bridge })
    }

    pub async fn init(&mut self) -> Result<()> {
        let interface = self.metadata.interface();
        let (connection, handle, _) = rtnetlink::new_connection()?;
        tokio::spawn(connection);

        let mut links = handle.link().get().match_name(interface.clone()).execute();
        let link = links.try_next().await?;
        if link.is_none() {
            return Err(anyhow!(
                "unable to find network interface named {}",
                interface
            ));
        }
        let link = link.unwrap();
        handle.link().set(link.header.index).up().execute().await?;
        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        let mut stack = self.create_network_stack().await?;
        loop {
            if !stack.poll().await? {
                break;
            }
        }
        Ok(())
    }

    async fn create_network_stack(&self) -> Result<NetworkStack> {
        let interface = self.metadata.interface();
        let proxy = Box::new(ProxyNatHandlerFactory::new());
        let addresses: Vec<IpCidr> = vec![
            self.metadata.gateway.ipv4.into(),
            self.metadata.gateway.ipv6.into(),
        ];
        let mut kdev =
            RawSocketHandle::bound_to_interface(&interface, RawSocketProtocol::Ethernet)?;
        let mtu = kdev.mtu_of_interface(&interface)? + EXTRA_MTU;
        let (tx_sender, tx_receiver) = channel::<BytesMut>(TX_CHANNEL_BUFFER_LEN);
        let mut udev = ChannelDevice::new(mtu, Medium::Ethernet, tx_sender.clone());
        let mac = self.metadata.gateway.mac;
        let local_cidrs = addresses.clone();
        let nat = Nat::new(mtu, proxy, mac, local_cidrs, tx_sender.clone())?;
        let hardware_addr = HardwareAddress::Ethernet(mac);
        let config = Config::new(hardware_addr);
        let mut iface = Interface::new(config, &mut udev, Instant::now());
        iface.update_ip_addrs(|addrs| {
            addrs
                .extend_from_slice(&addresses)
                .expect("failed to set ip addresses");
        });
        let sockets = SocketSet::new(vec![]);
        let handle = self.bridge.join(self.metadata.zone.mac).await?;
        let kdev = AsyncRawSocketChannel::new(mtu, kdev)?;
        Ok(NetworkStack {
            tx: tx_receiver,
            kdev,
            udev,
            interface: iface,
            sockets,
            nat,
            bridge: handle,
        })
    }

    pub async fn launch(self) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            info!(
                "launched network backend for krata zone {}",
                self.metadata.uuid
            );
            if let Err(error) = self.run().await {
                warn!(
                    "network backend for krata zone {} failed: {}",
                    self.metadata.uuid, error
                );
            }
        }))
    }
}

impl Drop for NetworkBackend {
    fn drop(&mut self) {
        info!(
            "destroyed network backend for krata zone {}",
            self.metadata.uuid
        );
    }
}
