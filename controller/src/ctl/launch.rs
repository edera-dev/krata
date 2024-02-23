use std::{fs, net::Ipv4Addr, str::FromStr};

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use ipnetwork::Ipv4Network;
use krata::{
    LaunchChannels, LaunchInfo, LaunchNetwork, LaunchNetworkIpv4, LaunchNetworkIpv6,
    LaunchNetworkResolver,
};
use uuid::Uuid;
use xenclient::{DomainConfig, DomainDisk, DomainEventChannel, DomainNetworkInterface};
use xenstore::client::XsdInterface;

use crate::image::{name::ImageName, ImageCompiler, ImageInfo};

use super::{cfgblk::ConfigBlock, ControllerContext};

pub struct ControllerLaunchRequest<'a> {
    pub kernel_path: &'a str,
    pub initrd_path: &'a str,
    pub image: &'a str,
    pub vcpus: u32,
    pub mem: u64,
    pub env: Option<Vec<String>>,
    pub run: Option<Vec<String>>,
    pub debug: bool,
}

pub struct ControllerLaunch<'a> {
    context: &'a mut ControllerContext,
}

impl ControllerLaunch<'_> {
    pub fn new(context: &mut ControllerContext) -> ControllerLaunch<'_> {
        ControllerLaunch { context }
    }

    pub async fn perform(&mut self, request: ControllerLaunchRequest<'_>) -> Result<(Uuid, u32)> {
        let uuid = Uuid::new_v4();
        let name = format!("krata-{uuid}");
        let image_info = self.compile(request.image)?;

        let mut gateway_mac = MacAddr6::random();
        gateway_mac.set_local(true);
        gateway_mac.set_multicast(false);
        let mut container_mac = MacAddr6::random();
        container_mac.set_local(true);
        container_mac.set_multicast(false);

        let guest_ipv4 = self.allocate_ipv4().await?;
        let guest_ipv6 = container_mac.to_link_local_ipv6();
        let gateway_ipv4 = "192.168.42.1";
        let gateway_ipv6 = "fe80::1";
        let ipv4_network_mask: u32 = 24;
        let ipv6_network_mask: u32 = 10;

        let launch_config = LaunchInfo {
            network: Some(LaunchNetwork {
                link: "eth0".to_string(),
                ipv4: LaunchNetworkIpv4 {
                    address: format!("{}/{}", guest_ipv4, ipv4_network_mask),
                    gateway: gateway_ipv4.to_string(),
                },
                ipv6: LaunchNetworkIpv6 {
                    address: format!("{}/{}", guest_ipv6, ipv6_network_mask),
                    gateway: gateway_ipv6.to_string(),
                },
                resolver: LaunchNetworkResolver {
                    nameservers: vec![
                        "1.1.1.1".to_string(),
                        "1.0.0.1".to_string(),
                        "2606:4700:4700::1111".to_string(),
                        "2606:4700:4700::1001".to_string(),
                    ],
                },
            }),
            env: request.env,
            run: request.run,
            channels: LaunchChannels {
                exit: "krata-exit".to_string(),
            },
        };

        let cfgblk = ConfigBlock::new(&uuid, &image_info)?;
        cfgblk.build(&launch_config)?;

        let image_squashfs_path = image_info
            .image_squashfs
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert image squashfs path to string"))?;

        let cfgblk_dir_path = cfgblk
            .dir
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk directory path to string"))?;
        let cfgblk_squashfs_path = cfgblk
            .file
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk squashfs path to string"))?;

        let image_squashfs_loop = self.context.autoloop.loopify(image_squashfs_path)?;
        let cfgblk_squashfs_loop = self.context.autoloop.loopify(cfgblk_squashfs_path)?;

        let cmdline_options = [
            if request.debug { "debug" } else { "quiet" },
            "elevator=noop",
        ];
        let cmdline = cmdline_options.join(" ");

        let container_mac_string = container_mac.to_string().replace('-', ":");
        let gateway_mac_string = gateway_mac.to_string().replace('-', ":");
        let config = DomainConfig {
            backend_domid: 0,
            name: &name,
            max_vcpus: request.vcpus,
            mem_mb: request.mem,
            kernel_path: request.kernel_path,
            initrd_path: request.initrd_path,
            cmdline: &cmdline,
            disks: vec![
                DomainDisk {
                    vdev: "xvda",
                    block: &image_squashfs_loop,
                    writable: false,
                },
                DomainDisk {
                    vdev: "xvdb",
                    block: &cfgblk_squashfs_loop,
                    writable: false,
                },
            ],
            consoles: vec![],
            vifs: vec![DomainNetworkInterface {
                mac: &container_mac_string,
                mtu: 1500,
                bridge: None,
                script: None,
            }],
            filesystems: vec![],
            event_channels: vec![DomainEventChannel { name: "krata-exit" }],
            extra_keys: vec![
                ("krata/uuid".to_string(), uuid.to_string()),
                (
                    "krata/loops".to_string(),
                    format!(
                        "{}:{}:none,{}:{}:{}",
                        &image_squashfs_loop.path,
                        image_squashfs_path,
                        &cfgblk_squashfs_loop.path,
                        cfgblk_squashfs_path,
                        cfgblk_dir_path,
                    ),
                ),
                ("krata/image".to_string(), request.image.to_string()),
                (
                    "krata/network/guest/ipv4".to_string(),
                    format!("{}/{}", guest_ipv4, ipv4_network_mask),
                ),
                (
                    "krata/network/guest/ipv6".to_string(),
                    format!("{}/{}", guest_ipv6, ipv6_network_mask),
                ),
                (
                    "krata/network/guest/mac".to_string(),
                    container_mac_string.clone(),
                ),
                (
                    "krata/network/gateway/ipv4".to_string(),
                    format!("{}/{}", gateway_ipv4, ipv4_network_mask),
                ),
                (
                    "krata/network/gateway/ipv6".to_string(),
                    format!("{}/{}", gateway_ipv6, ipv6_network_mask),
                ),
                (
                    "krata/network/gateway/mac".to_string(),
                    gateway_mac_string.clone(),
                ),
            ],
        };
        match self.context.xen.create(&config).await {
            Ok(domid) => Ok((uuid, domid)),
            Err(error) => {
                let _ = self.context.autoloop.unloop(&image_squashfs_loop.path);
                let _ = self.context.autoloop.unloop(&cfgblk_squashfs_loop.path);
                let _ = fs::remove_dir(&cfgblk.dir);
                Err(error.into())
            }
        }
    }

    async fn allocate_ipv4(&mut self) -> Result<Ipv4Addr> {
        let network = Ipv4Network::new(Ipv4Addr::new(192, 168, 42, 0), 24)?;
        let mut used: Vec<Ipv4Addr> = vec![
            Ipv4Addr::new(192, 168, 42, 0),
            Ipv4Addr::new(192, 168, 42, 1),
            Ipv4Addr::new(192, 168, 42, 255),
        ];
        for domid_candidate in self.context.xen.store.list("/local/domain").await? {
            let dom_path = format!("/local/domain/{}", domid_candidate);
            let ip_path = format!("{}/krata/network/guest/ipv4", dom_path);
            let existing_ip = self.context.xen.store.read_string(&ip_path).await?;
            if let Some(existing_ip) = existing_ip {
                let ipv4_network = Ipv4Network::from_str(&existing_ip)?;
                used.push(ipv4_network.ip());
            }
        }

        let mut found: Option<Ipv4Addr> = None;
        for ip in network.iter() {
            if !used.contains(&ip) {
                found = Some(ip);
                break;
            }
        }

        if found.is_none() {
            return Err(anyhow!(
                "unable to find ipv4 to allocate to container, ipv4 addresses are exhausted"
            ));
        }

        Ok(found.unwrap())
    }

    fn compile(&self, image: &str) -> Result<ImageInfo> {
        let image = ImageName::parse(image)?;
        let compiler = ImageCompiler::new(&self.context.image_cache)?;
        compiler.compile(&image)
    }
}
