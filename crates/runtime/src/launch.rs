use std::collections::HashMap;
use std::net::{IpAddr, Ipv6Addr};
use std::sync::Arc;
use std::{fs, net::Ipv4Addr, str::FromStr};

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use ipnetwork::{IpNetwork, Ipv4Network};
use krata::launchcfg::{
    LaunchInfo, LaunchNetwork, LaunchNetworkIpv4, LaunchNetworkIpv6, LaunchNetworkResolver,
    LaunchPackedFormat, LaunchRoot,
};
use krataoci::packer::OciImagePacked;
use tokio::sync::Semaphore;
use uuid::Uuid;
use xenclient::{DomainChannel, DomainConfig, DomainDisk, DomainNetworkInterface};
use xenstore::XsdInterface;

use crate::cfgblk::ConfigBlock;
use crate::RuntimeContext;

use super::{GuestInfo, GuestState};

pub struct GuestLaunchRequest {
    pub format: LaunchPackedFormat,
    pub uuid: Option<Uuid>,
    pub name: Option<String>,
    pub vcpus: u32,
    pub mem: u64,
    pub env: HashMap<String, String>,
    pub run: Option<Vec<String>>,
    pub debug: bool,
    pub image: OciImagePacked,
}

pub struct GuestLauncher {
    pub launch_semaphore: Arc<Semaphore>,
}

impl GuestLauncher {
    pub fn new(launch_semaphore: Arc<Semaphore>) -> Result<Self> {
        Ok(Self { launch_semaphore })
    }

    pub async fn launch(
        &mut self,
        context: &RuntimeContext,
        request: GuestLaunchRequest,
    ) -> Result<GuestInfo> {
        let uuid = request.uuid.unwrap_or_else(Uuid::new_v4);
        let xen_name = format!("krata-{uuid}");
        let mut gateway_mac = MacAddr6::random();
        gateway_mac.set_local(true);
        gateway_mac.set_multicast(false);
        let mut container_mac = MacAddr6::random();
        container_mac.set_local(true);
        container_mac.set_multicast(false);

        let _launch_permit = self.launch_semaphore.acquire().await?;
        let guest_ipv4 = self.allocate_ipv4(context).await?;
        let guest_ipv6 = container_mac.to_link_local_ipv6();
        let gateway_ipv4 = "10.75.70.1";
        let gateway_ipv6 = "fe80::1";
        let ipv4_network_mask: u32 = 16;
        let ipv6_network_mask: u32 = 10;

        let launch_config = LaunchInfo {
            root: LaunchRoot {
                format: request.format.clone(),
            },
            hostname: Some(
                request
                    .name
                    .as_ref()
                    .map(|x| x.to_string())
                    .unwrap_or_else(|| format!("krata-{}", uuid)),
            ),
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
        };

        let cfgblk = ConfigBlock::new(&uuid, &request.image)?;
        cfgblk.build(&launch_config)?;

        let image_squashfs_path = request
            .image
            .path
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert image path to string"))?;

        let cfgblk_dir_path = cfgblk
            .dir
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk directory path to string"))?;
        let cfgblk_squashfs_path = cfgblk
            .file
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk squashfs path to string"))?;

        let image_squashfs_loop = context.autoloop.loopify(image_squashfs_path)?;
        let cfgblk_squashfs_loop = context.autoloop.loopify(cfgblk_squashfs_path)?;

        let cmdline_options = [
            if request.debug { "debug" } else { "quiet" },
            "elevator=noop",
        ];
        let cmdline = cmdline_options.join(" ");

        let guest_mac_string = container_mac.to_string().replace('-', ":");
        let gateway_mac_string = gateway_mac.to_string().replace('-', ":");

        let mut extra_keys = vec![
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
                guest_mac_string.clone(),
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
        ];

        if let Some(name) = request.name.as_ref() {
            extra_keys.push(("krata/name".to_string(), name.clone()));
        }

        let config = DomainConfig {
            backend_domid: 0,
            name: &xen_name,
            max_vcpus: request.vcpus,
            mem_mb: request.mem,
            kernel_path: &context.kernel,
            initrd_path: &context.initrd,
            cmdline: &cmdline,
            use_console_backend: Some("krata-console"),
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
            channels: vec![DomainChannel {
                typ: "krata-channel".to_string(),
                initialized: false,
            }],
            vifs: vec![DomainNetworkInterface {
                mac: &guest_mac_string,
                mtu: 1500,
                bridge: None,
                script: None,
            }],
            filesystems: vec![],
            event_channels: vec![],
            extra_keys,
            extra_rw_paths: vec!["krata/guest".to_string()],
        };
        match context.xen.create(&config).await {
            Ok(created) => Ok(GuestInfo {
                name: request.name.as_ref().map(|x| x.to_string()),
                uuid,
                domid: created.domid,
                image: request.image.digest,
                loops: vec![],
                guest_ipv4: Some(IpNetwork::new(
                    IpAddr::V4(guest_ipv4),
                    ipv4_network_mask as u8,
                )?),
                guest_ipv6: Some(IpNetwork::new(
                    IpAddr::V6(guest_ipv6),
                    ipv6_network_mask as u8,
                )?),
                guest_mac: Some(guest_mac_string.clone()),
                gateway_ipv4: Some(IpNetwork::new(
                    IpAddr::V4(Ipv4Addr::from_str(gateway_ipv4)?),
                    ipv4_network_mask as u8,
                )?),
                gateway_ipv6: Some(IpNetwork::new(
                    IpAddr::V6(Ipv6Addr::from_str(gateway_ipv6)?),
                    ipv6_network_mask as u8,
                )?),
                gateway_mac: Some(gateway_mac_string.clone()),
                state: GuestState { exit_code: None },
            }),
            Err(error) => {
                let _ = context.autoloop.unloop(&image_squashfs_loop.path).await;
                let _ = context.autoloop.unloop(&cfgblk_squashfs_loop.path).await;
                let _ = fs::remove_dir(&cfgblk.dir);
                Err(error.into())
            }
        }
    }

    async fn allocate_ipv4(&self, context: &RuntimeContext) -> Result<Ipv4Addr> {
        let network = Ipv4Network::new(Ipv4Addr::new(10, 75, 80, 0), 24)?;
        let mut used: Vec<Ipv4Addr> = vec![];
        for domid_candidate in context.xen.store.list("/local/domain").await? {
            let dom_path = format!("/local/domain/{}", domid_candidate);
            let ip_path = format!("{}/krata/network/guest/ipv4", dom_path);
            let existing_ip = context.xen.store.read_string(&ip_path).await?;
            if let Some(existing_ip) = existing_ip {
                let ipv4_network = Ipv4Network::from_str(&existing_ip)?;
                used.push(ipv4_network.ip());
            }
        }

        let mut found: Option<Ipv4Addr> = None;
        for ip in network.iter() {
            let last = ip.octets()[3];
            if last == 0 || last == 255 {
                continue;
            }
            if !used.contains(&ip) {
                found = Some(ip);
                break;
            }
        }

        if found.is_none() {
            return Err(anyhow!(
                "unable to find ipv4 to allocate to guest, ipv4 addresses are exhausted"
            ));
        }

        Ok(found.unwrap())
    }
}
