use crate::image::cache::ImageCache;
use crate::image::name::ImageName;
use crate::image::registry::OciRegistryPlatform;
use anyhow::{anyhow, Result};
use backhand::compression::Compressor;
use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader};
use log::{debug, trace, warn};
use oci_spec::image::{ImageConfiguration, ImageManifest};
use std::fs::File;
use std::io::{BufReader, Cursor};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use tokio::fs;
use tokio::io::AsyncRead;
use tokio_stream::StreamExt;
use tokio_tar::{Archive, Entry};
use uuid::Uuid;
use walkdir::WalkDir;

use crate::image::fetch::{OciImageDownloader, OciImageLayer};

pub const IMAGE_SQUASHFS_VERSION: u64 = 1;
const LAYER_BUFFER_SIZE: usize = 128 * 1024;

// we utilize in-memory buffers when generating the squashfs for files
// under this size. for files of or above this size, we open a file.
// the file is then read during writing. we want to reduce the number
// of open files during squashfs generation, so this limit should be set
// to something that limits the number of files on average, at the expense
// of increased memory usage.
// TODO: it may be wise to, during crawling of the image layers, infer this
//       value from the size to file count ratio of all layers.
const SQUASHFS_MEMORY_BUFFER_LIMIT: usize = 8 * 1024 * 1024;

pub struct ImageInfo {
    pub image_squashfs: PathBuf,
    pub manifest: ImageManifest,
    pub config: ImageConfiguration,
}

impl ImageInfo {
    pub fn new(
        squashfs: PathBuf,
        manifest: ImageManifest,
        config: ImageConfiguration,
    ) -> Result<ImageInfo> {
        Ok(ImageInfo {
            image_squashfs: squashfs,
            manifest,
            config,
        })
    }
}

pub struct ImageCompiler<'a> {
    cache: &'a ImageCache,
    seed: Option<PathBuf>,
}

impl ImageCompiler<'_> {
    pub fn new(cache: &ImageCache, seed: Option<PathBuf>) -> Result<ImageCompiler> {
        Ok(ImageCompiler { cache, seed })
    }

    pub async fn compile(&self, image: &ImageName) -> Result<ImageInfo> {
        debug!("compile image={image}");
        let mut tmp_dir = std::env::temp_dir().clone();
        tmp_dir.push(format!("krata-compile-{}", Uuid::new_v4()));

        let mut image_dir = tmp_dir.clone();
        image_dir.push("image");
        fs::create_dir_all(&image_dir).await?;

        let mut layer_dir = tmp_dir.clone();
        layer_dir.push("layer");
        fs::create_dir_all(&layer_dir).await?;

        let mut squash_file = tmp_dir.clone();
        squash_file.push("image.squashfs");
        let info = self
            .download_and_compile(image, &layer_dir, &image_dir, &squash_file)
            .await?;
        fs::remove_dir_all(&tmp_dir).await?;
        Ok(info)
    }

    async fn download_and_compile(
        &self,
        image: &ImageName,
        layer_dir: &Path,
        image_dir: &Path,
        squash_file: &Path,
    ) -> Result<ImageInfo> {
        let downloader = OciImageDownloader::new(
            self.seed.clone(),
            layer_dir.to_path_buf(),
            OciRegistryPlatform::current(),
        );
        let resolved = downloader.resolve(image.clone()).await?;
        let cache_key = format!(
            "manifest={}:squashfs-version={}\n",
            resolved.digest, IMAGE_SQUASHFS_VERSION
        );
        let cache_digest = sha256::digest(cache_key);

        if let Some(cached) = self.cache.recall(&cache_digest).await? {
            return Ok(cached);
        }

        let local = downloader.download(resolved).await?;
        for layer in &local.layers {
            debug!(
                "process layer digest={} compression={:?}",
                &layer.digest, layer.compression,
            );
            self.process_layer_whiteout(layer, image_dir).await?;
            let mut archive = layer.archive().await?;
            let mut entries = archive.entries()?;
            while let Some(entry) = entries.next().await {
                let mut entry = entry?;
                let path = entry.path()?;
                let Some(name) = path.file_name() else {
                    return Err(anyhow!("unable to get file name"));
                };
                let Some(name) = name.to_str() else {
                    return Err(anyhow!("unable to get file name as string"));
                };

                if name.starts_with(".wh.") {
                    continue;
                } else {
                    self.process_write_entry(&mut entry, layer, image_dir)
                        .await?;
                }
            }
        }

        for layer in &local.layers {
            if layer.path.exists() {
                fs::remove_file(&layer.path).await?;
            }
        }

        self.squash(image_dir, squash_file)?;
        let info = ImageInfo::new(
            squash_file.to_path_buf(),
            local.image.manifest,
            local.config,
        )?;
        self.cache.store(&cache_digest, &info).await
    }

    async fn process_layer_whiteout(&self, layer: &OciImageLayer, image_dir: &Path) -> Result<()> {
        let mut archive = layer.archive().await?;
        let mut entries = archive.entries()?;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            let path = entry.path()?;
            let Some(name) = path.file_name() else {
                return Err(anyhow!("unable to get file name"));
            };
            let Some(name) = name.to_str() else {
                return Err(anyhow!("unable to get file name as string"));
            };

            if name.starts_with(".wh.") {
                self.process_whiteout_entry(&entry, name, layer, image_dir)
                    .await?;
            }
        }
        Ok(())
    }

    async fn process_whiteout_entry(
        &self,
        entry: &Entry<Archive<Pin<Box<dyn AsyncRead + Send>>>>,
        name: &str,
        layer: &OciImageLayer,
        image_dir: &Path,
    ) -> Result<()> {
        let dst = self.check_safe_entry(entry, image_dir)?;
        let mut dst = dst.clone();
        dst.pop();

        let opaque = name == ".wh..wh..opq";

        if !opaque {
            dst.push(&name[4..]);
            self.check_safe_path(&dst, image_dir)?;
        }

        trace!(
            "whiteout entry layer={} path={:?}",
            &layer.digest,
            entry.path()?
        );

        if opaque {
            if dst.is_dir() {
                let mut reader = fs::read_dir(dst).await?;
                while let Some(entry) = reader.next_entry().await? {
                    let path = entry.path();
                    if path.is_symlink() || path.is_file() {
                        fs::remove_file(&path).await?;
                    } else if path.is_dir() {
                        fs::remove_dir_all(&path).await?;
                    } else {
                        return Err(anyhow!("opaque whiteout entry did not exist"));
                    }
                }
            } else {
                warn!(
                    "whiteout opaque entry missing locally layer={} path={:?} local={:?}",
                    &layer.digest,
                    entry.path()?,
                    dst,
                );
            }
        } else if dst.is_file() || dst.is_symlink() {
            fs::remove_file(&dst).await?;
        } else if dst.is_dir() {
            fs::remove_dir_all(&dst).await?;
        } else {
            warn!(
                "whiteout entry missing locally layer={} path={:?} local={:?}",
                &layer.digest,
                entry.path()?,
                dst,
            );
        }
        Ok(())
    }

    async fn process_write_entry(
        &self,
        entry: &mut Entry<Archive<Pin<Box<dyn AsyncRead + Send>>>>,
        layer: &OciImageLayer,
        image_dir: &Path,
    ) -> Result<()> {
        trace!(
            "unpack entry layer={} path={:?} type={:?}",
            &layer.digest,
            entry.path()?,
            entry.header().entry_type()
        );
        entry.unpack_in(image_dir).await?;
        Ok(())
    }

    fn check_safe_entry(
        &self,
        entry: &Entry<Archive<Pin<Box<dyn AsyncRead + Send>>>>,
        image_dir: &Path,
    ) -> Result<PathBuf> {
        let mut dst = image_dir.to_path_buf();
        dst.push(entry.path()?);
        if let Some(name) = dst.file_name() {
            if let Some(name) = name.to_str() {
                if name.starts_with(".wh.") {
                    let copy = dst.clone();
                    dst.pop();
                    self.check_safe_path(&dst, image_dir)?;
                    return Ok(copy);
                }
            }
        }
        self.check_safe_path(&dst, image_dir)?;
        Ok(dst)
    }

    fn check_safe_path(&self, dst: &Path, image_dir: &Path) -> Result<()> {
        let resolved = path_clean::clean(dst);
        if !resolved.starts_with(image_dir) {
            return Err(anyhow!("layer attempts to work outside image dir"));
        }
        Ok(())
    }

    fn squash(&self, image_dir: &Path, squash_file: &Path) -> Result<()> {
        let mut writer = FilesystemWriter::default();
        writer.set_compressor(FilesystemCompressor::new(Compressor::Gzip, None)?);
        let walk = WalkDir::new(image_dir).follow_links(false);
        for entry in walk {
            let entry = entry?;
            let rel = entry
                .path()
                .strip_prefix(image_dir)?
                .to_str()
                .ok_or_else(|| anyhow!("failed to strip prefix of tmpdir"))?;
            let rel = format!("/{}", rel);
            trace!("squash write {}", rel);
            let typ = entry.file_type();
            let metadata = std::fs::symlink_metadata(entry.path())?;
            let uid = metadata.uid();
            let gid = metadata.gid();
            let mode = metadata.permissions().mode();
            let mtime = metadata.mtime();

            if rel == "/" {
                writer.set_root_uid(uid);
                writer.set_root_gid(gid);
                writer.set_root_mode(mode as u16);
                continue;
            }

            let header = NodeHeader {
                permissions: mode as u16,
                uid,
                gid,
                mtime: mtime as u32,
            };
            if typ.is_symlink() {
                let symlink = std::fs::read_link(entry.path())?;
                let symlink = symlink
                    .to_str()
                    .ok_or_else(|| anyhow!("failed to read symlink"))?;
                writer.push_symlink(symlink, rel, header)?;
            } else if typ.is_dir() {
                writer.push_dir(rel, header)?;
            } else if typ.is_file() {
                if metadata.size() >= SQUASHFS_MEMORY_BUFFER_LIMIT as u64 {
                    let reader =
                        BufReader::with_capacity(LAYER_BUFFER_SIZE, File::open(entry.path())?);
                    writer.push_file(reader, rel, header)?;
                } else {
                    let cursor = Cursor::new(std::fs::read(entry.path())?);
                    writer.push_file(cursor, rel, header)?;
                }
            } else if typ.is_block_device() {
                let device = metadata.dev();
                writer.push_block_device(device as u32, rel, header)?;
            } else if typ.is_char_device() {
                let device = metadata.dev();
                writer.push_char_device(device as u32, rel, header)?;
            } else {
                return Err(anyhow!("invalid file type"));
            }
        }

        std::fs::remove_dir_all(image_dir)?;

        let squash_file_path = squash_file
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert squashfs string"))?;

        let mut file = File::create(squash_file)?;
        trace!("squash generate: {}", squash_file_path);
        writer.write(&mut file)?;
        Ok(())
    }
}
