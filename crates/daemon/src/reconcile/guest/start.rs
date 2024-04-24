use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use futures::StreamExt;
use krata::launchcfg::LaunchPackedFormat;
use krata::v1::common::GuestOciImageSpec;
use krata::v1::common::{guest_image_spec::Image, Guest, GuestState, GuestStatus, OciImageFormat};
use krataoci::packer::{service::OciPackerService, OciPackedFormat};
use kratart::{launch::GuestLaunchRequest, Runtime};
use log::info;

use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;
use tokio_tar::Archive;
use uuid::Uuid;

use crate::{
    glt::GuestLookupTable,
    reconcile::guest::{guestinfo_to_networkstate, GuestReconcilerResult},
};

// if a kernel is >= 100MB, that's kinda scary.
const OCI_SPEC_TAR_FILE_MAX_SIZE: usize = 100 * 1024 * 1024;

pub struct GuestStarter<'a> {
    pub kernel_path: &'a Path,
    pub initrd_path: &'a Path,
    pub packer: &'a OciPackerService,
    pub glt: &'a GuestLookupTable,
    pub runtime: &'a Runtime,
}

impl GuestStarter<'_> {
    pub async fn oci_spec_tar_read_file(
        &self,
        file: &Path,
        oci: &GuestOciImageSpec,
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
            if entry.header().size()? as usize > OCI_SPEC_TAR_FILE_MAX_SIZE {
                return Err(anyhow!(
                    "file {} in image {} is larger than the size limit",
                    file.to_string_lossy(),
                    oci.digest
                ));
            }
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

    pub async fn start(&self, uuid: Uuid, guest: &mut Guest) -> Result<GuestReconcilerResult> {
        let Some(ref spec) = guest.spec else {
            return Err(anyhow!("guest spec not specified"));
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
                        return Err(anyhow!("tar image format is not supported for guests"));
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

        let info = self
            .runtime
            .launch(GuestLaunchRequest {
                format: LaunchPackedFormat::Squashfs,
                uuid: Some(uuid),
                name: if spec.name.is_empty() {
                    None
                } else {
                    Some(spec.name.clone())
                },
                image,
                kernel,
                initrd,
                vcpus: spec.vcpus,
                mem: spec.mem,
                pcis: vec![],
                env: task
                    .environment
                    .iter()
                    .map(|x| (x.key.clone(), x.value.clone()))
                    .collect::<HashMap<_, _>>(),
                run: empty_vec_optional(task.command.clone()),
                debug: false,
            })
            .await?;
        self.glt.associate(uuid, info.domid).await;
        info!("started guest {}", uuid);
        guest.state = Some(GuestState {
            status: GuestStatus::Started.into(),
            network: Some(guestinfo_to_networkstate(&info)),
            exit_info: None,
            error_info: None,
            host: self.glt.host_uuid().to_string(),
            domid: info.domid,
        });
        Ok(GuestReconcilerResult::Changed { rerun: false })
    }
}

fn empty_vec_optional<T>(value: Vec<T>) -> Option<Vec<T>> {
    if value.is_empty() {
        None
    } else {
        Some(value)
    }
}
