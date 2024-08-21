use anyhow::{anyhow, Result};
use futures::StreamExt;
use krata::launchcfg::LaunchPackedFormat;
use krata::v1::common::{OciImageFormat, Zone, ZoneState, ZoneStatus};
use krata::v1::common::{ZoneOciImageSpec, ZoneResourceStatus};
use krataoci::packer::{service::OciPackerService, OciPackedFormat};
use kratart::launch::{PciBdf, PciDevice, PciRdmReservePolicy, ZoneLaunchNetwork};
use kratart::{launch::ZoneLaunchRequest, Runtime};
use log::info;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::{DaemonConfig, DaemonPciDeviceRdmReservePolicy};
use crate::devices::DaemonDeviceManager;
use crate::ip::assignment::IpAssignment;
use crate::reconcile::zone::ip_reservation_to_network_status;
use crate::{reconcile::zone::ZoneReconcilerResult, zlt::ZoneLookupTable};
use krata::v1::common::zone_image_spec::Image;
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;
use tokio_tar::Archive;
use uuid::Uuid;

pub struct ZoneCreator<'a> {
    pub devices: &'a DaemonDeviceManager,
    pub kernel_path: &'a Path,
    pub initrd_path: &'a Path,
    pub addons_path: &'a Path,
    pub packer: &'a OciPackerService,
    pub ip_assignment: &'a IpAssignment,
    pub zlt: &'a ZoneLookupTable,
    pub runtime: &'a Runtime,
    pub config: &'a DaemonConfig,
}

impl ZoneCreator<'_> {
    pub async fn oci_spec_tar_read_file(
        &self,
        file: &Path,
        oci: &ZoneOciImageSpec,
    ) -> Result<Vec<u8>> {
        if oci.format() != OciImageFormat::Tar {
            return Err(anyhow!(
                "oci image spec for {} is required to be in tar format",
                oci.digest
            ));
        }

        let image = self
            .packer
            .recall(&oci.digest, OciPackedFormat::Tar)
            .await?;

        let Some(image) = image else {
            return Err(anyhow!("image {} was not found in tar format", oci.digest));
        };

        let mut archive = Archive::new(File::open(&image.path).await?);
        let mut entries = archive.entries()?;
        while let Some(entry) = entries.next().await {
            let mut entry = entry?;
            let path = entry.path()?;
            if path == file {
                let mut buffer = Vec::new();
                entry.read_to_end(&mut buffer).await?;
                return Ok(buffer);
            }
        }
        Err(anyhow!(
            "unable to find file {} in image {}",
            file.to_string_lossy(),
            oci.digest
        ))
    }

    pub async fn create(&self, uuid: Uuid, zone: &mut Zone) -> Result<ZoneReconcilerResult> {
        let Some(ref mut spec) = zone.spec else {
            return Err(anyhow!("zone spec not specified"));
        };

        let Some(ref image) = spec.image else {
            return Err(anyhow!("image spec not provided"));
        };
        let oci = match image.image {
            Some(Image::Oci(ref oci)) => oci,
            None => {
                return Err(anyhow!("oci spec not specified"));
            }
        };
        let task = spec.task.as_ref().cloned().unwrap_or_default();

        let image = self
            .packer
            .recall(
                &oci.digest,
                match oci.format() {
                    OciImageFormat::Unknown => OciPackedFormat::Squashfs,
                    OciImageFormat::Squashfs => OciPackedFormat::Squashfs,
                    OciImageFormat::Erofs => OciPackedFormat::Erofs,
                    OciImageFormat::Tar => {
                        return Err(anyhow!("tar image format is not supported for zones"));
                    }
                },
            )
            .await?;

        let Some(image) = image else {
            return Err(anyhow!(
                "image {} in the requested format did not exist",
                oci.digest
            ));
        };

        let kernel = if let Some(ref spec) = spec.kernel {
            let Some(Image::Oci(ref oci)) = spec.image else {
                return Err(anyhow!("kernel image spec must be an oci image"));
            };
            self.oci_spec_tar_read_file(&PathBuf::from("kernel/image"), oci)
                .await?
        } else {
            fs::read(&self.kernel_path).await?
        };
        let initrd = if let Some(ref spec) = spec.initrd {
            let Some(Image::Oci(ref oci)) = spec.image else {
                return Err(anyhow!("initrd image spec must be an oci image"));
            };
            self.oci_spec_tar_read_file(&PathBuf::from("krata/initrd"), oci)
                .await?
        } else {
            fs::read(&self.initrd_path).await?
        };

        let success = AtomicBool::new(false);

        let _device_release_guard = scopeguard::guard(
            (spec.devices.clone(), self.devices.clone()),
            |(devices, manager)| {
                if !success.load(Ordering::Acquire) {
                    tokio::task::spawn(async move {
                        for device in devices {
                            let _ = manager.release(&device.name, uuid).await;
                        }
                    });
                }
            },
        );

        let mut pcis = Vec::new();
        for device in &spec.devices {
            let state = self.devices.claim(&device.name, uuid).await?;
            if let Some(cfg) = state.pci {
                for location in cfg.locations {
                    let pci = PciDevice {
                        bdf: PciBdf::from_str(&location)?.with_domain(0),
                        permissive: cfg.permissive,
                        msi_translate: cfg.msi_translate,
                        power_management: cfg.power_management,
                        rdm_reserve_policy: match cfg.rdm_reserve_policy {
                            DaemonPciDeviceRdmReservePolicy::Strict => PciRdmReservePolicy::Strict,
                            DaemonPciDeviceRdmReservePolicy::Relaxed => {
                                PciRdmReservePolicy::Relaxed
                            }
                        },
                    };
                    pcis.push(pci);
                }
            } else {
                return Err(anyhow!(
                    "device '{}' isn't a known device type",
                    device.name
                ));
            }
        }

        let reservation = self.ip_assignment.assign(uuid).await?;

        let mut initial_resources = spec.initial_resources.unwrap_or_default();
        if initial_resources.target_cpus < 1 {
            initial_resources.target_cpus = 1;
        }
        if initial_resources.target_cpus > initial_resources.max_cpus {
            initial_resources.max_cpus = initial_resources.target_cpus;
        }
        spec.initial_resources = Some(initial_resources);
        let kernel_options = spec.kernel_options.clone().unwrap_or_default();
        let info = self
            .runtime
            .launch(ZoneLaunchRequest {
                format: match image.format {
                    OciPackedFormat::Squashfs => LaunchPackedFormat::Squashfs,
                    OciPackedFormat::Erofs => LaunchPackedFormat::Erofs,
                    _ => {
                        return Err(anyhow!(
                            "oci image is in an invalid format, which isn't compatible with launch"
                        ));
                    }
                },
                uuid: Some(uuid),
                name: if spec.name.is_empty() {
                    None
                } else {
                    Some(spec.name.clone())
                },
                image,
                kernel,
                initrd,
                target_cpus: initial_resources.target_cpus,
                max_cpus: initial_resources.max_cpus,
                max_memory: initial_resources.max_memory,
                target_memory: initial_resources.target_memory,
                pcis,
                env: task
                    .environment
                    .iter()
                    .map(|x| (x.key.clone(), x.value.clone()))
                    .collect::<HashMap<_, _>>(),
                run: empty_vec_optional(task.command.clone()),
                kernel_verbose: kernel_options.verbose,
                kernel_cmdline_append: kernel_options.cmdline_append,
                addons_image: Some(self.addons_path.to_path_buf()),
                network: ZoneLaunchNetwork {
                    ipv4: reservation.ipv4.to_string(),
                    ipv4_prefix: reservation.ipv4_prefix,
                    ipv6: reservation.ipv6.to_string(),
                    ipv6_prefix: reservation.ipv6_prefix,
                    gateway_ipv4: reservation.gateway_ipv4.to_string(),
                    gateway_ipv6: reservation.gateway_ipv6.to_string(),
                    zone_mac: reservation.mac,
                    nameservers: self.config.network.nameservers.clone(),
                },
            })
            .await?;
        self.zlt.associate(uuid, info.domid).await;
        info!("created zone {}", uuid);
        zone.status = Some(ZoneStatus {
            state: ZoneState::Created.into(),
            network_status: Some(ip_reservation_to_network_status(&reservation)),
            exit_status: None,
            error_status: None,
            resource_status: Some(ZoneResourceStatus {
                active_resources: Some(initial_resources),
            }),
            host: self.zlt.host_uuid().to_string(),
            domid: info.domid,
        });
        success.store(true, Ordering::Release);
        Ok(ZoneReconcilerResult::Changed { rerun: false })
    }
}

fn empty_vec_optional<T>(value: Vec<T>) -> Option<Vec<T>> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
