use crate::{
    fetch::OciResolvedImage,
    packer::{OciImagePacked, OciPackedFormat},
};

use anyhow::Result;
use log::debug;
use oci_spec::image::{ImageConfiguration, ImageManifest};
use std::path::{Path, PathBuf};
use tokio::fs;

#[derive(Clone)]
pub struct OciPackerCache {
    cache_dir: PathBuf,
}

impl OciPackerCache {
    pub fn new(cache_dir: &Path) -> Result<OciPackerCache> {
        Ok(OciPackerCache {
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    pub async fn recall(
        &self,
        resolved: &OciResolvedImage,
        format: OciPackedFormat,
    ) -> Result<Option<OciImagePacked>> {
        let mut fs_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        fs_path.push(format!("{}.{}", resolved.digest, format.extension()));
        manifest_path.push(format!("{}.manifest.json", resolved.digest));
        config_path.push(format!("{}.config.json", resolved.digest));
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
                    debug!("cache hit digest={}", resolved.digest);
                    Some(OciImagePacked::new(
                        resolved.digest.clone(),
                        fs_path.clone(),
                        format,
                        config,
                        manifest,
                    ))
                } else {
                    None
                }
            } else {
                debug!("cache miss digest={}", resolved.digest);
                None
            },
        )
    }

    pub async fn store(&self, packed: OciImagePacked) -> Result<OciImagePacked> {
        debug!("cache store digest={}", packed.digest);
        let mut fs_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        fs_path.push(format!("{}.{}", packed.digest, packed.format.extension()));
        manifest_path.push(format!("{}.manifest.json", packed.digest));
        config_path.push(format!("{}.config.json", packed.digest));
        fs::copy(&packed.path, &fs_path).await?;
        let manifest_text = serde_json::to_string_pretty(&packed.manifest)?;
        fs::write(&manifest_path, manifest_text).await?;
        let config_text = serde_json::to_string_pretty(&packed.config)?;
        fs::write(&config_path, config_text).await?;
        Ok(OciImagePacked::new(
            packed.digest,
            fs_path.clone(),
            packed.format,
            packed.config,
            packed.manifest,
        ))
    }
}
