use crate::{
    progress::{OciBoundProgress, OciProgressPhase},
    schema::OciSchema,
};

use super::{
    name::ImageName,
    registry::{OciPlatform, OciRegistryClient},
};

use std::{
    fmt::Debug,
    io::SeekFrom,
    os::unix::fs::MetadataExt,
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
    fs::{self, File},
    io::{AsyncRead, AsyncReadExt, AsyncSeekExt, BufReader, BufWriter},
};
use tokio_stream::StreamExt;
use tokio_tar::Archive;

pub struct OciImageFetcher {
    seed: Option<PathBuf>,
    platform: OciPlatform,
    progress: OciBoundProgress,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OciImageLayerCompression {
    None,
    Gzip,
    Zstd,
}

#[derive(Clone, Debug)]
pub struct OciImageLayer {
    pub metadata: Descriptor,
    pub path: PathBuf,
    pub digest: String,
    pub compression: OciImageLayerCompression,
}

#[async_trait::async_trait]
pub trait OciImageLayerReader: AsyncRead + Sync {
    async fn position(&mut self) -> Result<u64>;
}

#[async_trait::async_trait]
impl OciImageLayerReader for BufReader<File> {
    async fn position(&mut self) -> Result<u64> {
        Ok(self.seek(SeekFrom::Current(0)).await?)
    }
}

#[async_trait::async_trait]
impl OciImageLayerReader for GzipDecoder<BufReader<File>> {
    async fn position(&mut self) -> Result<u64> {
        self.get_mut().position().await
    }
}

#[async_trait::async_trait]
impl OciImageLayerReader for ZstdDecoder<BufReader<File>> {
    async fn position(&mut self) -> Result<u64> {
        self.get_mut().position().await
    }
}

impl OciImageLayer {
    pub async fn decompress(&self) -> Result<Pin<Box<dyn OciImageLayerReader + Send>>> {
        let file = File::open(&self.path).await?;
        let reader = BufReader::new(file);
        let reader: Pin<Box<dyn OciImageLayerReader + Send>> = match self.compression {
            OciImageLayerCompression::None => Box::pin(reader),
            OciImageLayerCompression::Gzip => Box::pin(GzipDecoder::new(reader)),
            OciImageLayerCompression::Zstd => Box::pin(ZstdDecoder::new(reader)),
        };
        Ok(reader)
    }

    pub async fn archive(&self) -> Result<Archive<Pin<Box<dyn OciImageLayerReader + Send>>>> {
        let decompress = self.decompress().await?;
        Ok(Archive::new(decompress))
    }
}

#[derive(Clone, Debug)]
pub struct OciResolvedImage {
    pub name: ImageName,
    pub digest: String,
    pub manifest: OciSchema<ImageManifest>,
}

#[derive(Clone, Debug)]
pub struct OciLocalImage {
    pub image: OciResolvedImage,
    pub config: OciSchema<ImageConfiguration>,
    pub layers: Vec<OciImageLayer>,
}

impl OciImageFetcher {
    pub fn new(
        seed: Option<PathBuf>,
        platform: OciPlatform,
        progress: OciBoundProgress,
    ) -> OciImageFetcher {
        OciImageFetcher {
            seed,
            platform,
            progress,
        }
    }

    async fn load_seed_json_blob<T: Clone + Debug + DeserializeOwned>(
        &self,
        descriptor: &Descriptor,
    ) -> Result<Option<OciSchema<T>>> {
        let digest = descriptor.digest();
        let Some((digest_type, digest_content)) = digest.split_once(':') else {
            return Err(anyhow!("digest content was not properly formatted"));
        };
        let want = format!("blobs/{}/{}", digest_type, digest_content);
        self.load_seed_json(&want).await
    }

    async fn load_seed_json<T: Clone + Debug + DeserializeOwned>(
        &self,
        want: &str,
    ) -> Result<Option<OciSchema<T>>> {
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
                let mut content = Vec::new();
                entry.read_to_end(&mut content).await?;
                let item = serde_json::from_slice::<T>(&content)?;
                return Ok(Some(OciSchema::new(content, item)));
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
            for manifest in index.item().manifests() {
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

    pub async fn download(
        &self,
        image: &OciResolvedImage,
        layer_dir: &Path,
    ) -> Result<OciLocalImage> {
        let config: OciSchema<ImageConfiguration>;
        self.progress
            .update(|progress| {
                progress.phase = OciProgressPhase::ConfigDownload;
            })
            .await;
        let mut client = OciRegistryClient::new(image.name.registry_url()?, self.platform.clone())?;
        if let Some(seeded) = self
            .load_seed_json_blob::<ImageConfiguration>(image.manifest.item().config())
            .await?
        {
            config = seeded;
        } else {
            let config_bytes = client
                .get_blob(&image.name.name, image.manifest.item().config())
                .await?;
            config = OciSchema::new(
                config_bytes.to_vec(),
                serde_json::from_slice(&config_bytes)?,
            );
        }
        self.progress
            .update(|progress| {
                progress.phase = OciProgressPhase::LayerDownload;

                for layer in image.manifest.item().layers() {
                    progress.add_layer(layer.digest());
                }
            })
            .await;
        let mut layers = Vec::new();
        for layer in image.manifest.item().layers() {
            self.progress
                .update(|progress| {
                    progress.downloading_layer(layer.digest(), 0, layer.size() as u64);
                })
                .await;
            layers.push(
                self.acquire_layer(&image.name, layer, layer_dir, &mut client)
                    .await?,
            );
            self.progress
                .update(|progress| {
                    progress.downloaded_layer(layer.digest(), layer.size() as u64);
                })
                .await;
        }
        Ok(OciLocalImage {
            image: image.clone(),
            config,
            layers,
        })
    }

    async fn acquire_layer(
        &self,
        image: &ImageName,
        layer: &Descriptor,
        layer_dir: &Path,
        client: &mut OciRegistryClient,
    ) -> Result<OciImageLayer> {
        debug!(
            "acquire layer digest={} size={}",
            layer.digest(),
            layer.size()
        );
        let mut layer_path = layer_dir.to_path_buf();
        layer_path.push(format!("{}.layer", layer.digest()));

        let seeded = self.extract_seed_blob(layer, &layer_path).await?;
        if !seeded {
            let file = File::create(&layer_path).await?;
            let size = client
                .write_blob_to_file(&image.name, layer, file, Some(self.progress.clone()))
                .await?;
            if layer.size() as u64 != size {
                return Err(anyhow!(
                    "downloaded layer size differs from size in manifest",
                ));
            }
        }

        let metadata = fs::metadata(&layer_path).await?;

        if layer.size() as u64 != metadata.size() {
            return Err(anyhow!("layer size differs from size in manifest",));
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
            metadata: layer.clone(),
            path: layer_path,
            digest: layer.digest().clone(),
            compression,
        })
    }
}
