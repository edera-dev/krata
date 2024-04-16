use std::path::PathBuf;

use crate::{name::ImageName, schema::OciSchema};

use self::backend::OciPackerBackendType;
use oci_spec::image::{Descriptor, ImageConfiguration, ImageManifest};

pub mod backend;
pub mod cache;
pub mod service;

#[derive(Debug, Default, Clone, Copy, Eq, PartialEq, Hash)]
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
pub struct OciPackedImage {
    pub name: ImageName,
    pub digest: String,
    pub path: PathBuf,
    pub format: OciPackedFormat,
    pub descriptor: Descriptor,
    pub config: OciSchema<ImageConfiguration>,
    pub manifest: OciSchema<ImageManifest>,
}

impl OciPackedImage {
    pub fn new(
        name: ImageName,
        digest: String,
        path: PathBuf,
        format: OciPackedFormat,
        descriptor: Descriptor,
        config: OciSchema<ImageConfiguration>,
        manifest: OciSchema<ImageManifest>,
    ) -> OciPackedImage {
        OciPackedImage {
            name,
            digest,
            path,
            format,
            descriptor,
            config,
            manifest,
        }
    }
}
