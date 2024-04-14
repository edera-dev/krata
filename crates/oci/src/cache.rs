use crate::packer::OciPackerFormat;

use super::compiler::ImageInfo;
use anyhow::Result;
use log::debug;
use oci_spec::image::{ImageConfiguration, ImageManifest};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Clone)]
pub struct ImageCache {
    cache_dir: PathBuf,
}

impl ImageCache {
    pub fn new(cache_dir: &Path) -> Result<ImageCache> {
        Ok(ImageCache {
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    pub async fn recall(&self, digest: &str, format: OciPackerFormat) -> Result<Option<ImageInfo>> {
        let mut fs_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        fs_path.push(format!("{}.{}", digest, format.extension()));
        manifest_path.push(format!("{}.manifest.json", digest));
        config_path.push(format!("{}.config.json", digest));
        Ok(
            if fs_path.exists() && manifest_path.exists() && config_path.exists() {
                let image_metadata = fs::metadata(&fs_path).await?;
                let manifest_metadata = fs::metadata(&manifest_path).await?;
                let config_metadata = fs::metadata(&config_path).await?;
                if image_metadata.is_file()
                    && manifest_metadata.is_file()
                    && config_metadata.is_file()
                {
                    let manifest_text = fs::read_to_string(&manifest_path).await?;
                    let manifest: ImageManifest = serde_json::from_str(&manifest_text)?;
                    let config_text = fs::read_to_string(&config_path).await?;
                    let config: ImageConfiguration = serde_json::from_str(&config_text)?;
                    debug!("cache hit digest={}", digest);
                    Some(ImageInfo::new(fs_path.clone(), manifest, config)?)
                } else {
                    None
                }
            } else {
                debug!("cache miss digest={}", digest);
                None
            },
        )
    }

    pub async fn store(
        &self,
        digest: &str,
        info: &ImageInfo,
        format: OciPackerFormat,
    ) -> Result<ImageInfo> {
        debug!("cache store digest={}", digest);
        let mut fs_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        fs_path.push(format!("{}.{}", digest, format.extension()));
        manifest_path.push(format!("{}.manifest.json", digest));
        config_path.push(format!("{}.config.json", digest));
        fs::copy(&info.image, &fs_path).await?;
        let manifest_text = serde_json::to_string_pretty(&info.manifest)?;
        fs::write(&manifest_path, manifest_text).await?;
        let config_text = serde_json::to_string_pretty(&info.config)?;
        fs::write(&config_path, config_text).await?;
        ImageInfo::new(fs_path.clone(), info.manifest.clone(), info.config.clone())
    }
}
