use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;
use std::sync::Arc;

use advmac::MacAddr6;
use anyhow::{anyhow, Result};
use tokio::sync::Semaphore;
use uuid::Uuid;

use krata::launchcfg::{
    LaunchInfo, LaunchNetwork, LaunchNetworkIpv4, LaunchNetworkIpv6, LaunchNetworkResolver,
    LaunchPackedFormat, LaunchRoot,
};
use krataoci::packer::OciPackedImage;
pub use xenclient::{
    pci::PciBdf, DomainPciDevice as PciDevice, DomainPciRdmReservePolicy as PciRdmReservePolicy,
};
use xenclient::{DomainChannel, DomainConfig, DomainDisk, DomainNetworkInterface};
use xenplatform::domain::BaseDomainConfig;

use crate::cfgblk::ConfigBlock;
use crate::RuntimeContext;

use super::{ZoneInfo, ZoneState};

pub struct ZoneLaunchRequest {
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
    pub network: ZoneLaunchNetwork,
}

pub struct ZoneLaunchNetwork {
    pub ipv4: String,
    pub ipv4_prefix: u8,
    pub ipv6: String,
    pub ipv6_prefix: u8,
    pub gateway_ipv4: String,
    pub gateway_ipv6: String,
    pub zone_mac: MacAddr6,
    pub nameservers: Vec<String>,
}

pub struct ZoneLauncher {
    pub launch_semaphore: Arc<Semaphore>,
}

impl ZoneLauncher {
    pub fn new(launch_semaphore: Arc<Semaphore>) -> Result<Self> {
        Ok(Self { launch_semaphore })
    }

    pub async fn launch(
        &mut self,
        context: &RuntimeContext,
        request: ZoneLaunchRequest,
    ) -> Result<ZoneInfo> {
        let uuid = request.uuid.unwrap_or_else(Uuid::new_v4);
        let xen_name = format!("krata-{uuid}");
        let _launch_permit = self.launch_semaphore.acquire().await?;
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
                    address: format!("{}/{}", request.network.ipv4, request.network.ipv4_prefix),
                    gateway: request.network.gateway_ipv4,
                },
                ipv6: LaunchNetworkIpv6 {
                    address: format!("{}/{}", request.network.ipv6, request.network.ipv6_prefix),
                    gateway: request.network.gateway_ipv6.to_string(),
                },
                resolver: LaunchNetworkResolver {
                    nameservers: request.network.nameservers,
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
        let mut cmdline_options = ["console=hvc0"].to_vec();
        if !request.debug {
            cmdline_options.push("quiet");
        }
        let cmdline = cmdline_options.join(" ");

        let zone_mac_string = request.network.zone_mac.to_string().replace('-', ":");

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
        ];

        if let Some(name) = request.name.as_ref() {
            extra_keys.push(("krata/name".to_string(), name.clone()));
        }

        let config = DomainConfig {
            base: BaseDomainConfig {
                max_vcpus: request.vcpus,
                mem_mb: request.mem,
                kernel: request.kernel,
                initrd: request.initrd,
                cmdline,
                uuid,
                owner_domid: 0,
                enable_iommu: !request.pcis.is_empty(),
            },
            backend_domid: 0,
            name: xen_name,
            swap_console_backend: Some("krata-console".to_string()),
            disks,
            channels: vec![DomainChannel {
                typ: "krata-channel".to_string(),
                initialized: false,
            }],
            vifs: vec![DomainNetworkInterface {
                mac: zone_mac_string.clone(),
                mtu: 1500,
                bridge: None,
                script: None,
            }],
            pcis: request.pcis.clone(),
            filesystems: vec![],
            extra_keys,
            extra_rw_paths: vec!["krata/zone".to_string()],
        };
        match context.xen.create(&config).await {
            Ok(created) => Ok(ZoneInfo {
                name: request.name.as_ref().map(|x| x.to_string()),
                uuid,
                domid: created.domid,
                image: request.image.digest,
                loops: vec![],
                state: ZoneState { exit_code: None },
            }),
            Err(error) => {
                let _ = context.autoloop.unloop(&image_squashfs_loop.path).await;
                let _ = context.autoloop.unloop(&cfgblk_squashfs_loop.path).await;
                let _ = fs::remove_dir(&cfgblk_dir);
                Err(error.into())
            }
        }
    }
}
