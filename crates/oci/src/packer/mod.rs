use std::path::PathBuf;

use self::backend::OciPackerBackendType;
use oci_spec::image::{ImageConfiguration, ImageManifest};

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

    pub fn backend(&self) -> OciPackerBackendType {
        match self {
            OciPackedFormat::Squashfs => OciPackerBackendType::MkSquashfs,
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
