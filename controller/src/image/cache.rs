use crate::image::{ImageInfo, Result};
use log::debug;
use oci_spec::image::{ImageConfiguration, ImageManifest};
use std::fs;
use std::path::{Path, PathBuf};

pub struct ImageCache {
    cache_dir: PathBuf,
}

impl ImageCache {
    pub fn new(cache_dir: &Path) -> Result<ImageCache> {
        Ok(ImageCache {
            cache_dir: cache_dir.to_path_buf(),
        })
    }

    pub fn recall(&self, digest: &str) -> Result<Option<ImageInfo>> {
        let mut squashfs_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        squashfs_path.push(format!("{}.squashfs", digest));
        manifest_path.push(format!("{}.manifest.json", digest));
        config_path.push(format!("{}.config.json", digest));
        Ok(
            if squashfs_path.exists() && manifest_path.exists() && config_path.exists() {
                let squashfs_metadata = fs::metadata(&squashfs_path)?;
                let manifest_metadata = fs::metadata(&manifest_path)?;
                let config_metadata = fs::metadata(&config_path)?;
                if squashfs_metadata.is_file()
                    && manifest_metadata.is_file()
                    && config_metadata.is_file()
                {
                    let manifest_text = fs::read_to_string(&manifest_path)?;
                    let manifest: ImageManifest = serde_json::from_str(&manifest_text)?;
                    let config_text = fs::read_to_string(&config_path)?;
                    let config: ImageConfiguration = serde_json::from_str(&config_text)?;
                    debug!("cache hit digest={}", digest);
                    Some(ImageInfo::new(squashfs_path.clone(), manifest, config)?)
                } else {
                    None
                }
            } else {
                debug!("cache miss digest={}", digest);
                None
            },
        )
    }

    pub fn store(&self, digest: &str, info: &ImageInfo) -> Result<ImageInfo> {
        debug!("cache store digest={}", digest);
        let mut squashfs_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        squashfs_path.push(format!("{}.squashfs", digest));
        manifest_path.push(format!("{}.manifest.json", digest));
        config_path.push(format!("{}.config.json", digest));
        fs::copy(&info.image_squashfs, &squashfs_path)?;
        let manifest_text = serde_json::to_string_pretty(&info.manifest)?;
        fs::write(&manifest_path, manifest_text)?;
        let config_text = serde_json::to_string_pretty(&info.config)?;
        fs::write(&config_path, config_text)?;
        ImageInfo::new(
            squashfs_path.clone(),
            info.manifest.clone(),
            info.config.clone(),
        )
    }
}
