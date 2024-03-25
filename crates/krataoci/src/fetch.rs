use super::{
    name::ImageName,
    registry::{OciRegistryClient, OciRegistryPlatform},
};

use std::{
    path::{Path, PathBuf},
    pin::Pin,
};

use anyhow::{anyhow, Result};
use async_compression::tokio::bufread::{GzipDecoder, ZstdDecoder};
use log::debug;
use oci_spec::image::{
    Descriptor, ImageConfiguration, ImageIndex, ImageManifest, MediaType, ToDockerV2S2,
};
use serde::de::DeserializeOwned;
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncReadExt, BufReader, BufWriter},
};
use tokio_stream::StreamExt;
use tokio_tar::Archive;

pub struct OciImageDownloader {
    seed: Option<PathBuf>,
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
    pub fn new(
        seed: Option<PathBuf>,
        storage: PathBuf,
        platform: OciRegistryPlatform,
    ) -> OciImageDownloader {
        OciImageDownloader {
            seed,
            storage,
            platform,
        }
    }

    async fn load_seed_json_blob<T: DeserializeOwned>(
        &self,
        descriptor: &Descriptor,
    ) -> Result<Option<T>> {
        let digest = descriptor.digest();
        let Some((digest_type, digest_content)) = digest.split_once(':') else {
            return Err(anyhow!("digest content was not properly formatted"));
        };
        let want = format!("blobs/{}/{}", digest_type, digest_content);
        self.load_seed_json(&want).await
    }

    async fn load_seed_json<T: DeserializeOwned>(&self, want: &str) -> Result<Option<T>> {
        let Some(ref seed) = self.seed else {
            return Ok(None);
        };

        let file = File::open(seed).await?;
        let mut archive = Archive::new(file);
        let mut entries = archive.entries()?;
        while let Some(entry) = entries.next().await {
            let mut entry = entry?;
            let path = String::from_utf8(entry.path_bytes().to_vec())?;
            if path == want {
                let mut content = String::new();
                entry.read_to_string(&mut content).await?;
                let data = serde_json::from_str::<T>(&content)?;
                return Ok(Some(data));
            }
        }
        Ok(None)
    }

    async fn extract_seed_blob(&self, descriptor: &Descriptor, to: &Path) -> Result<bool> {
        let Some(ref seed) = self.seed else {
            return Ok(false);
        };

        let digest = descriptor.digest();
        let Some((digest_type, digest_content)) = digest.split_once(':') else {
            return Err(anyhow!("digest content was not properly formatted"));
        };
        let want = format!("blobs/{}/{}", digest_type, digest_content);

        let seed = File::open(seed).await?;
        let mut archive = Archive::new(seed);
        let mut entries = archive.entries()?;
        while let Some(entry) = entries.next().await {
            let mut entry = entry?;
            let path = String::from_utf8(entry.path_bytes().to_vec())?;
            if path == want {
                let file = File::create(to).await?;
                let mut bufwrite = BufWriter::new(file);
                tokio::io::copy(&mut entry, &mut bufwrite).await?;
                return Ok(true);
            }
        }
        Ok(false)
    }

    pub async fn resolve(&self, image: ImageName) -> Result<OciResolvedImage> {
        debug!("resolve manifest image={}", image);

        if let Some(index) = self.load_seed_json::<ImageIndex>("index.json").await? {
            let mut found: Option<&Descriptor> = None;
            for manifest in index.manifests() {
                let Some(annotations) = manifest.annotations() else {
                    continue;
                };

                let mut image_name = annotations.get("io.containerd.image.name");
                if image_name.is_none() {
                    image_name = annotations.get("org.opencontainers.image.ref.name");
                }

                let Some(image_name) = image_name else {
                    continue;
                };

                if *image_name != image.to_string() {
                    continue;
                }

                if let Some(platform) = manifest.platform() {
                    if *platform.architecture() != self.platform.arch
                        || *platform.os() != self.platform.os
                    {
                        continue;
                    }
                }
                found = Some(manifest);
                break;
            }

            if let Some(found) = found {
                if let Some(manifest) = self.load_seed_json_blob(found).await? {
                    debug!(
                        "found seeded manifest image={} manifest={}",
                        image,
                        found.digest()
                    );
                    return Ok(OciResolvedImage {
                        name: image,
                        digest: found.digest().clone(),
                        manifest,
                    });
                }
            }
        }

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
        let config: ImageConfiguration;

        let mut client = OciRegistryClient::new(image.name.registry_url()?, self.platform.clone())?;
        if let Some(seeded) = self
            .load_seed_json_blob::<ImageConfiguration>(image.manifest.config())
            .await?
        {
            config = seeded;
        } else {
            let config_bytes = client
                .get_blob(&image.name.name, image.manifest.config())
                .await?;
            config = serde_json::from_slice(&config_bytes)?;
        }
        let mut layers = Vec::new();
        for layer in image.manifest.layers() {
            layers.push(self.acquire_layer(&image.name, layer, &mut client).await?);
        }
        Ok(OciLocalImage {
            image,
            config,
            layers,
        })
    }

    async fn acquire_layer(
        &self,
        image: &ImageName,
        layer: &Descriptor,
        client: &mut OciRegistryClient,
    ) -> Result<OciImageLayer> {
        debug!(
            "acquire layer digest={} size={}",
            layer.digest(),
            layer.size()
        );
        let mut layer_path = self.storage.clone();
        layer_path.push(format!("{}.layer", layer.digest()));

        let seeded = self.extract_seed_blob(layer, &layer_path).await?;
        if !seeded {
            let file = File::create(&layer_path).await?;
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
