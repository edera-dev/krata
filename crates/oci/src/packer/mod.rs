use std::path::PathBuf;

use crate::schema::OciSchema;

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
    Tar,
}

impl OciPackedFormat {
    pub fn extension(&self) -> &str {
        match self {
            OciPackedFormat::Squashfs => "squashfs",
            OciPackedFormat::Erofs => "erofs",
            OciPackedFormat::Tar => "tar",
        }
    }

    pub fn backend(&self) -> OciPackerBackendType {
        match self {
            OciPackedFormat::Squashfs => OciPackerBackendType::MkSquashfs,
            OciPackedFormat::Erofs => OciPackerBackendType::MkfsErofs,
            OciPackedFormat::Tar => OciPackerBackendType::Tar,
        }
    }
}

#[derive(Clone)]
pub struct OciImagePacked {
    pub digest: String,
    pub path: PathBuf,
    pub format: OciPackedFormat,
    pub config: OciSchema<ImageConfiguration>,
    pub manifest: OciSchema<ImageManifest>,
}

impl OciImagePacked {
    pub fn new(
        digest: String,
        path: PathBuf,
        format: OciPackedFormat,
        config: OciSchema<ImageConfiguration>,
        manifest: OciSchema<ImageManifest>,
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
