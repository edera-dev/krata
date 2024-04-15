use self::backend::OciPackerBackendType;
use oci_spec::image::{ImageConfiguration, ImageManifest};
use std::{
    path::PathBuf,
    process::{Command, Stdio},
};

pub mod backend;
pub mod cache;
pub mod service;

#[derive(Debug, Default, Clone, Copy)]
pub enum OciPackedFormat {
    #[default]
    Squashfs,
    Erofs,
}

impl OciPackedFormat {
    pub fn extension(&self) -> &str {
        match self {
            OciPackedFormat::Squashfs => "squashfs",
            OciPackedFormat::Erofs => "erofs",
        }
    }

    pub fn detect_best_backend(&self) -> OciPackerBackendType {
        match self {
            OciPackedFormat::Squashfs => {
                let status = Command::new("mksquashfs")
                    .arg("-version")
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .stdout(Stdio::null())
                    .status()
                    .ok();

                let Some(code) = status.and_then(|x| x.code()) else {
                    return OciPackerBackendType::Backhand;
                };

                if code == 0 {
                    OciPackerBackendType::MkSquashfs
                } else {
                    OciPackerBackendType::Backhand
                }
            }
            OciPackedFormat::Erofs => OciPackerBackendType::MkfsErofs,
        }
    }
}

#[derive(Clone)]
pub struct OciImagePacked {
    pub digest: String,
    pub path: PathBuf,
    pub format: OciPackedFormat,
    pub config: ImageConfiguration,
    pub manifest: ImageManifest,
}

impl OciImagePacked {
    pub fn new(
        digest: String,
        path: PathBuf,
        format: OciPackedFormat,
        config: ImageConfiguration,
        manifest: ImageManifest,
    ) -> OciImagePacked {
        OciImagePacked {
            digest,
            path,
            format,
            config,
            manifest,
        }
    }
}
