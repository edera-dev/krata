use std::{path::PathBuf, pin::Pin};

use anyhow::{anyhow, Result};
use async_compression::tokio::bufread::{GzipDecoder, ZstdDecoder};
use log::debug;
use oci_spec::image::{Descriptor, ImageConfiguration, ImageManifest, MediaType, ToDockerV2S2};
use tokio::{
    fs::File,
    io::{AsyncRead, BufReader},
};
use tokio_tar::Archive;

use super::{
    name::ImageName,
    registry::{OciRegistryClient, OciRegistryPlatform},
};

pub struct OciImageDownloader {
    storage: PathBuf,
    platform: OciRegistryPlatform,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OciImageLayerCompression {
    None,
    Gzip,
    Zstd,
}

#[derive(Clone, Debug)]
pub struct OciImageLayer {
    pub path: PathBuf,
    pub digest: String,
    pub compression: OciImageLayerCompression,
}

impl OciImageLayer {
    pub async fn decompress(&self) -> Result<Pin<Box<dyn AsyncRead + Send>>> {
        let file = File::open(&self.path).await?;
        let reader = BufReader::new(file);
        let reader: Pin<Box<dyn AsyncRead + Send>> = match self.compression {
            OciImageLayerCompression::None => Box::pin(reader),
            OciImageLayerCompression::Gzip => Box::pin(GzipDecoder::new(reader)),
            OciImageLayerCompression::Zstd => Box::pin(ZstdDecoder::new(reader)),
        };
        Ok(reader)
    }

    pub async fn archive(&self) -> Result<Archive<Pin<Box<dyn AsyncRead + Send>>>> {
        let decompress = self.decompress().await?;
        Ok(Archive::new(decompress))
    }
}

#[derive(Clone, Debug)]
pub struct OciResolvedImage {
    pub name: ImageName,
    pub digest: String,
    pub manifest: ImageManifest,
}

#[derive(Clone, Debug)]
pub struct OciLocalImage {
    pub image: OciResolvedImage,
    pub config: ImageConfiguration,
    pub layers: Vec<OciImageLayer>,
}

impl OciImageDownloader {
    pub fn new(storage: PathBuf, platform: OciRegistryPlatform) -> OciImageDownloader {
        OciImageDownloader { storage, platform }
    }

    pub async fn resolve(&self, image: ImageName) -> Result<OciResolvedImage> {
        debug!("download manifest image={}", image);
        let mut client = OciRegistryClient::new(image.registry_url()?, self.platform.clone())?;
        let (manifest, digest) = client
            .get_manifest_with_digest(&image.name, &image.reference)
            .await?;
        Ok(OciResolvedImage {
            name: image,
            digest,
            manifest,
        })
    }

    pub async fn download(&self, image: OciResolvedImage) -> Result<OciLocalImage> {
        let mut client = OciRegistryClient::new(image.name.registry_url()?, self.platform.clone())?;
        let config_bytes = client
            .get_blob(&image.name.name, image.manifest.config())
            .await?;
        let config: ImageConfiguration = serde_json::from_slice(&config_bytes)?;
        let mut layers = Vec::new();
        for layer in image.manifest.layers() {
            layers.push(self.download_layer(&image.name, layer, &mut client).await?);
        }
        Ok(OciLocalImage {
            image,
            config,
            layers,
        })
    }

    async fn download_layer(
        &self,
        image: &ImageName,
        layer: &Descriptor,
        client: &mut OciRegistryClient,
    ) -> Result<OciImageLayer> {
        debug!(
            "download layer digest={} size={}",
            layer.digest(),
            layer.size()
        );
        let mut layer_path = self.storage.clone();
        layer_path.push(format!("{}.layer", layer.digest()));

        {
            let file = tokio::fs::File::create(&layer_path).await?;
            let size = client.write_blob_to_file(&image.name, layer, file).await?;
            if layer.size() as u64 != size {
                return Err(anyhow!(
                    "downloaded layer size differs from size in manifest",
                ));
            }
        }

        let mut media_type = layer.media_type().clone();

        // docker layer compatibility
        if media_type.to_string() == MediaType::ImageLayerGzip.to_docker_v2s2()? {
            media_type = MediaType::ImageLayerGzip;
        }

        let compression = match media_type {
            MediaType::ImageLayer => OciImageLayerCompression::None,
            MediaType::ImageLayerGzip => OciImageLayerCompression::Gzip,
            MediaType::ImageLayerZstd => OciImageLayerCompression::Zstd,
            other => return Err(anyhow!("found layer with unknown media type: {}", other)),
        };
        Ok(OciImageLayer {
            path: layer_path,
            digest: layer.digest().clone(),
            compression,
        })
    }
}
