use crate::{
    packer::{OciImagePacked, OciPackedFormat},
    schema::OciSchema,
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
        digest: &str,
        format: OciPackedFormat,
    ) -> Result<Option<OciImagePacked>> {
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
                    let manifest_bytes = fs::read(&manifest_path).await?;
                    let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes)?;
                    let config_bytes = fs::read(&config_path).await?;
                    let config: ImageConfiguration = serde_json::from_slice(&config_bytes)?;
                    debug!("cache hit digest={}", digest);
                    Some(OciImagePacked::new(
                        digest.to_string(),
                        fs_path.clone(),
                        format,
                        OciSchema::new(config_bytes, config),
                        OciSchema::new(manifest_bytes, manifest),
                    ))
                } else {
                    None
                }
            } else {
                debug!("cache miss digest={}", digest);
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
        fs::rename(&packed.path, &fs_path).await?;
        fs::write(&config_path, packed.config.raw()).await?;
        fs::write(&manifest_path, packed.manifest.raw()).await?;
        Ok(OciImagePacked::new(
            packed.digest,
            fs_path.clone(),
            packed.format,
            packed.config,
            packed.manifest,
        ))
    }
}
