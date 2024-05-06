use std::collections::HashMap;
use std::fs;
use std::net::IpAddr;
use std::path::PathBuf;
use std::sync::Arc;

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use ipnetwork::IpNetwork;
use krata::launchcfg::{
    LaunchInfo, LaunchNetwork, LaunchNetworkIpv4, LaunchNetworkIpv6, LaunchNetworkResolver,
    LaunchPackedFormat, LaunchRoot,
};
use krataoci::packer::OciPackedImage;
use tokio::sync::Semaphore;
use uuid::Uuid;
use xenclient::{DomainChannel, DomainConfig, DomainDisk, DomainNetworkInterface};

use crate::cfgblk::ConfigBlock;
use crate::RuntimeContext;

use super::{GuestInfo, GuestState};

pub use xenclient::{
    pci::PciBdf, DomainPciDevice as PciDevice, DomainPciRdmReservePolicy as PciRdmReservePolicy,
};

pub struct GuestLaunchRequest {
    pub format: LaunchPackedFormat,
    pub kernel: Vec<u8>,
    pub initrd: Vec<u8>,
    pub uuid: Option<Uuid>,
    pub name: Option<String>,
    pub vcpus: u32,
    pub mem: u64,
    pub env: HashMap<String, String>,
    pub run: Option<Vec<String>>,
    pub pcis: Vec<PciDevice>,
    pub debug: bool,
    pub image: OciPackedImage,
    pub addons_image: Option<PathBuf>,
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
        let mut ip = context.ipvendor.assign(uuid).await?;
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
                    address: format!("{}/{}", ip.ipv4, ip.ipv4_prefix),
                    gateway: ip.gateway_ipv4.to_string(),
                },
                ipv6: LaunchNetworkIpv6 {
                    address: format!("{}/{}", ip.ipv6, ip.ipv6_prefix),
                    gateway: ip.gateway_ipv6.to_string(),
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

        let cfgblk = ConfigBlock::new(&uuid, request.image.clone())?;
        let cfgblk_file = cfgblk.file.clone();
        let cfgblk_dir = cfgblk.dir.clone();
        tokio::task::spawn_blocking(move || cfgblk.build(&launch_config)).await??;

        let image_squashfs_path = request
            .image
            .path
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert image path to string"))?;

        let cfgblk_dir_path = cfgblk_dir
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk directory path to string"))?;
        let cfgblk_squashfs_path = cfgblk_file
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert cfgblk squashfs path to string"))?;
        let addons_squashfs_path = request
            .addons_image
            .map(|x| x.to_str().map(|x| x.to_string()))
            .map(|x| {
                Some(x.ok_or_else(|| anyhow!("failed to convert addons squashfs path to string")))
            })
            .unwrap_or(None);

        let addons_squashfs_path = if let Some(path) = addons_squashfs_path {
            Some(path?)
        } else {
            None
        };

        let image_squashfs_loop = context.autoloop.loopify(image_squashfs_path)?;
        let cfgblk_squashfs_loop = context.autoloop.loopify(cfgblk_squashfs_path)?;
        let addons_squashfs_loop = if let Some(ref addons_squashfs_path) = addons_squashfs_path {
            Some(context.autoloop.loopify(addons_squashfs_path)?)
        } else {
            None
        };
        let cmdline_options = [
            if request.debug { "debug" } else { "quiet" },
            "elevator=noop",
        ];
        let cmdline = cmdline_options.join(" ");

        let guest_mac_string = container_mac.to_string().replace('-', ":");
        let gateway_mac_string = gateway_mac.to_string().replace('-', ":");

        let mut disks = vec![
            DomainDisk {
                vdev: "xvda".to_string(),
                block: image_squashfs_loop.clone(),
                writable: false,
            },
            DomainDisk {
                vdev: "xvdb".to_string(),
                block: cfgblk_squashfs_loop.clone(),
                writable: false,
            },
        ];

        if let Some(ref addons) = addons_squashfs_loop {
            disks.push(DomainDisk {
                vdev: "xvdc".to_string(),
                block: addons.clone(),
                writable: false,
            });
        }

        let mut loops = vec![
            format!("{}:{}:none", image_squashfs_loop.path, image_squashfs_path),
            format!(
                "{}:{}:{}",
                cfgblk_squashfs_loop.path, cfgblk_squashfs_path, cfgblk_dir_path
            ),
        ];

        if let Some(ref addons) = addons_squashfs_loop {
            loops.push(format!(
                "{}:{}:none",
                addons.path,
                addons_squashfs_path
                    .clone()
                    .ok_or_else(|| anyhow!("addons squashfs path missing"))?
            ));
        }

        let mut extra_keys = vec![
            ("krata/uuid".to_string(), uuid.to_string()),
            ("krata/loops".to_string(), loops.join(",")),
            (
                "krata/network/guest/ipv4".to_string(),
                format!("{}/{}", ip.ipv4, ip.ipv4_prefix),
            ),
            (
                "krata/network/guest/ipv6".to_string(),
                format!("{}/{}", ip.ipv6, ip.ipv6_prefix),
            ),
            (
                "krata/network/guest/mac".to_string(),
                guest_mac_string.clone(),
            ),
            (
                "krata/network/gateway/ipv4".to_string(),
                format!("{}/{}", ip.gateway_ipv4, ip.ipv4_prefix),
            ),
            (
                "krata/network/gateway/ipv6".to_string(),
                format!("{}/{}", ip.gateway_ipv6, ip.ipv6_prefix),
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
            name: xen_name,
            max_vcpus: request.vcpus,
            mem_mb: request.mem,
            kernel: request.kernel,
            initrd: request.initrd,
            cmdline,
            swap_console_backend: Some("krata-console".to_string()),
            disks,
            channels: vec![DomainChannel {
                typ: "krata-channel".to_string(),
                initialized: false,
            }],
            vifs: vec![DomainNetworkInterface {
                mac: guest_mac_string.clone(),
                mtu: 1500,
                bridge: None,
                script: None,
            }],
            pcis: request.pcis.clone(),
            filesystems: vec![],
            event_channels: vec![],
            extra_keys,
            extra_rw_paths: vec!["krata/guest".to_string()],
        };
        match context.xen.create(&config).await {
            Ok(created) => {
                ip.commit().await?;
                Ok(GuestInfo {
                    name: request.name.as_ref().map(|x| x.to_string()),
                    uuid,
                    domid: created.domid,
                    image: request.image.digest,
                    loops: vec![],
                    guest_ipv4: Some(IpNetwork::new(IpAddr::V4(ip.ipv4), ip.ipv4_prefix)?),
                    guest_ipv6: Some(IpNetwork::new(IpAddr::V6(ip.ipv6), ip.ipv6_prefix)?),
                    guest_mac: Some(guest_mac_string.clone()),
                    gateway_ipv4: Some(IpNetwork::new(
                        IpAddr::V4(ip.gateway_ipv4),
                        ip.ipv4_prefix,
                    )?),
                    gateway_ipv6: Some(IpNetwork::new(
                        IpAddr::V6(ip.gateway_ipv6),
                        ip.ipv6_prefix,
                    )?),
                    gateway_mac: Some(gateway_mac_string.clone()),
                    state: GuestState { exit_code: None },
                })
            }
            Err(error) => {
                let _ = context.autoloop.unloop(&image_squashfs_loop.path).await;
                let _ = context.autoloop.unloop(&cfgblk_squashfs_loop.path).await;
                let _ = fs::remove_dir(&cfgblk_dir);
                Err(error.into())
            }
        }
    }
}
