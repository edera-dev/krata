use crate::{
    name::ImageName,
    packer::{OciPackedFormat, OciPackedImage},
    schema::OciSchema,
};

use anyhow::Result;
use log::{debug, error};
use oci_spec::image::{
    Descriptor, ImageConfiguration, ImageIndex, ImageIndexBuilder, ImageManifest, MediaType,
    ANNOTATION_REF_NAME,
};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use tokio::{fs, sync::RwLock};

#[derive(Clone)]
pub struct OciPackerCache {
    cache_dir: PathBuf,
    index: Arc<RwLock<ImageIndex>>,
}

const ANNOTATION_IMAGE_NAME: &str = "io.containerd.image.name";
const ANNOTATION_OCI_PACKER_FORMAT: &str = "dev.krata.oci.packer.format";

impl OciPackerCache {
    pub async fn new(cache_dir: &Path) -> Result<OciPackerCache> {
        let index = ImageIndexBuilder::default()
            .schema_version(2u32)
            .media_type(MediaType::ImageIndex)
            .manifests(Vec::new())
            .build()?;
        let cache = OciPackerCache {
            cache_dir: cache_dir.to_path_buf(),
            index: Arc::new(RwLock::new(index)),
        };

        {
            let mut mutex = cache.index.write().await;
            *mutex = cache.load_index().await?;
        }

        Ok(cache)
    }

    pub async fn list(&self) -> Result<Vec<Descriptor>> {
        let index = self.index.read().await;
        Ok(index.manifests().clone())
    }

    pub async fn recall(
        &self,
        name: ImageName,
        digest: &str,
        format: OciPackedFormat,
    ) -> Result<Option<OciPackedImage>> {
        let index = self.index.read().await;

        let mut descriptor: Option<Descriptor> = None;
        for manifest in index.manifests() {
            if manifest.digest() == digest
                && manifest
                    .annotations()
                    .as_ref()
                    .and_then(|x| x.get(ANNOTATION_OCI_PACKER_FORMAT))
                    .map(|x| x.as_str())
                    == Some(format.extension())
            {
                descriptor = Some(manifest.clone());
                break;
            }
        }

        let Some(descriptor) = descriptor else {
            return Ok(None);
        };

        let mut fs_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        fs_path.push(format!("{}.{}", digest, format.extension()));
        manifest_path.push(format!("{}.manifest.json", digest));
        config_path.push(format!("{}.config.json", digest));

        if fs_path.exists() && manifest_path.exists() && config_path.exists() {
            let image_metadata = fs::metadata(&fs_path).await?;
            let manifest_metadata = fs::metadata(&manifest_path).await?;
            let config_metadata = fs::metadata(&config_path).await?;
            if image_metadata.is_file() && manifest_metadata.is_file() && config_metadata.is_file()
            {
                let manifest_bytes = fs::read(&manifest_path).await?;
                let manifest: ImageManifest = serde_json::from_slice(&manifest_bytes)?;
                let config_bytes = fs::read(&config_path).await?;
                let config: ImageConfiguration = serde_json::from_slice(&config_bytes)?;
                debug!("cache hit digest={}", digest);
                Ok(Some(OciPackedImage::new(
                    name,
                    digest.to_string(),
                    fs_path.clone(),
                    format,
                    descriptor,
                    OciSchema::new(config_bytes, config),
                    OciSchema::new(manifest_bytes, manifest),
                )))
            } else {
                Ok(None)
            }
        } else {
            debug!("cache miss digest={}", digest);
            Ok(None)
        }
    }

    pub async fn store(&self, packed: OciPackedImage) -> Result<OciPackedImage> {
        let mut index = self.index.write().await;
        let mut manifests = index.manifests().clone();
        debug!("cache store digest={}", packed.digest);
        let mut fs_path = self.cache_dir.clone();
        let mut manifest_path = self.cache_dir.clone();
        let mut config_path = self.cache_dir.clone();
        fs_path.push(format!("{}.{}", packed.digest, packed.format.extension()));
        manifest_path.push(format!("{}.manifest.json", packed.digest));
        config_path.push(format!("{}.config.json", packed.digest));
        if fs::rename(&packed.path, &fs_path).await.is_err() {
            fs::copy(&packed.path, &fs_path).await?;
            fs::remove_file(&packed.path).await?;
        }
        fs::write(&config_path, packed.config.raw()).await?;
        fs::write(&manifest_path, packed.manifest.raw()).await?;
        manifests.retain(|item| {
            if item.digest() != &packed.digest {
                return true;
            }

            let Some(format) = item
                .annotations()
                .as_ref()
                .and_then(|x| x.get(ANNOTATION_OCI_PACKER_FORMAT))
                .map(|x| x.as_str())
            else {
                return true;
            };

            if format != packed.format.extension() {
                return true;
            }

            false
        });

        let mut descriptor = packed.descriptor.clone();
        let mut annotations = descriptor.annotations().clone().unwrap_or_default();
        annotations.insert(
            ANNOTATION_OCI_PACKER_FORMAT.to_string(),
            packed.format.extension().to_string(),
        );
        let image_name = packed.name.to_string();
        annotations.insert(ANNOTATION_IMAGE_NAME.to_string(), image_name);
        let image_ref = packed.name.reference.clone();
        annotations.insert(ANNOTATION_REF_NAME.to_string(), image_ref);
        descriptor.set_annotations(Some(annotations));
        manifests.push(descriptor.clone());
        index.set_manifests(manifests);
        self.save_index(&index).await?;

        let packed = OciPackedImage::new(
            packed.name,
            packed.digest,
            fs_path.clone(),
            packed.format,
            descriptor,
            packed.config,
            packed.manifest,
        );
        Ok(packed)
    }

    async fn save_empty_index(&self) -> Result<ImageIndex> {
        let index = ImageIndexBuilder::default()
            .schema_version(2u32)
            .media_type(MediaType::ImageIndex)
            .manifests(Vec::new())
            .build()?;
        self.save_index(&index).await?;
        Ok(index)
    }

    async fn load_index(&self) -> Result<ImageIndex> {
        let mut index_path = self.cache_dir.clone();
        index_path.push("index.json");

        if !index_path.exists() {
            self.save_empty_index().await?;
        }

        let content = fs::read_to_string(&index_path).await?;
        let index = match serde_json::from_str::<ImageIndex>(&content) {
            Ok(index) => index,
            Err(error) => {
                error!("image index was corrupted, creating a new one: {}", error);
                self.save_empty_index().await?
            }
        };

        Ok(index)
    }

    async fn save_index(&self, index: &ImageIndex) -> Result<()> {
        let mut encoded = serde_json::to_string_pretty(index)?;
        encoded.push('\n');
        let mut index_path = self.cache_dir.clone();
        index_path.push("index.json");
        fs::write(&index_path, encoded).await?;
        Ok(())
    }
}
