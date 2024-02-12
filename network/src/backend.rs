use crate::autonet::NetworkMetadata;
use crate::chandev::ChannelDevice;
use crate::nat::NatRouter;
use crate::pkt::RecvPacket;
use crate::proxynat::ProxyNatHandlerFactory;
use crate::raw_socket::{AsyncRawSocket, RawSocketProtocol};
use crate::vbridge::{BridgeJoinHandle, VirtualBridge};
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use etherparse::SlicedPacket;
use futures::TryStreamExt;
use log::{debug, info, trace, warn};
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::Medium;
use smoltcp::time::Instant;
use smoltcp::wire::{HardwareAddress, IpCidr};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::select;
use tokio::sync::mpsc::{channel, Receiver};

#[derive(Clone)]
pub struct NetworkBackend {
    metadata: NetworkMetadata,
    bridge: VirtualBridge,
}

enum NetworkStackSelect<'a> {
    Receive(&'a [u8]),
    Send(Option<BytesMut>),
    Reclaim,
}

struct NetworkStack<'a> {
    mtu: usize,
    tx: Receiver<BytesMut>,
    kdev: AsyncRawSocket,
    udev: ChannelDevice,
    interface: Interface,
    sockets: SocketSet<'a>,
    router: NatRouter,
    bridge: BridgeJoinHandle,
}

impl NetworkStack<'_> {
    async fn poll(&mut self, buffer: &mut [u8]) -> Result<()> {
        let what = select! {
            x = self.kdev.read(buffer) => NetworkStackSelect::Receive(&buffer[0..x?]),
            x = self.bridge.bridge_rx_receiver.recv() => NetworkStackSelect::Send(x),
            x = self.bridge.broadcast_rx_receiver.recv() => NetworkStackSelect::Send(x.ok()),
            x = self.tx.recv() => NetworkStackSelect::Send(x),
            _ = self.router.process_reclaim() => NetworkStackSelect::Reclaim,
        };

        match what {
            NetworkStackSelect::Receive(packet) => {
                if let Err(error) = self.bridge.bridge_tx_sender.try_send(packet.into()) {
                    trace!("failed to send guest packet to bridge: {}", error);
                }

                let slice = SlicedPacket::from_ethernet(packet)?;
                let packet = RecvPacket::new(packet, &slice)?;
                if let Err(error) = self.router.process(&packet).await {
                    debug!("router failed to process packet: {}", error);
                }

                self.udev.rx = Some(packet.raw.into());
                self.interface
                    .poll(Instant::now(), &mut self.udev, &mut self.sockets);
            }

            NetworkStackSelect::Send(Some(packet)) => self.kdev.write_all(&packet).await?,

            NetworkStackSelect::Send(None) => {}

            NetworkStackSelect::Reclaim => {}
        }

        Ok(())
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
        tokio::time::sleep(Duration::from_secs(3)).await;
        Ok(())
    }

    pub async fn run(&self) -> Result<()> {
        let mut stack = self.create_network_stack().await?;
        let mut buffer = vec![0u8; stack.mtu];
        loop {
            stack.poll(&mut buffer).await?;
        }
    }

    async fn create_network_stack(&self) -> Result<NetworkStack> {
        let interface = self.metadata.interface();
        let proxy = Box::new(ProxyNatHandlerFactory::new());
        let addresses: Vec<IpCidr> = vec![
            self.metadata.gateway.ipv4.into(),
            self.metadata.gateway.ipv6.into(),
        ];
        let mut kdev = AsyncRawSocket::bound_to_interface(&interface, RawSocketProtocol::Ethernet)?;
        let mtu = kdev.mtu_of_interface(&interface)?;
        let (tx_sender, tx_receiver) = channel::<BytesMut>(100);
        let mut udev = ChannelDevice::new(mtu, Medium::Ethernet, tx_sender.clone());
        let mac = self.metadata.gateway.mac;
        let nat = NatRouter::new(mtu, proxy, mac, addresses.clone(), tx_sender.clone());
        let hardware_addr = HardwareAddress::Ethernet(mac);
        let config = Config::new(hardware_addr);
        let mut iface = Interface::new(config, &mut udev, Instant::now());
        iface.update_ip_addrs(|addrs| {
            addrs
                .extend_from_slice(&addresses)
                .expect("failed to set ip addresses");
        });
        let sockets = SocketSet::new(vec![]);
        let handle = self.bridge.join(self.metadata.guest.mac).await?;
        Ok(NetworkStack {
            mtu,
            tx: tx_receiver,
            kdev,
            udev,
            interface: iface,
            sockets,
            router: nat,
            bridge: handle,
        })
    }

    pub async fn launch(self) -> Result<()> {
        tokio::task::spawn(async move {
            info!(
                "lauched network backend for hypha guest {}",
                self.metadata.uuid
            );
            if let Err(error) = self.run().await {
                warn!(
                    "network backend for hypha guest {} failed: {}",
                    self.metadata.uuid, error
                );
            }
        });
        Ok(())
    }
}
