use crate::cache::ImageCache;
use crate::fetch::{OciImageDownloader, OciImageLayer};
use crate::name::ImageName;
use crate::registry::OciRegistryPlatform;
use anyhow::{anyhow, Result};
use backhand::compression::Compressor;
use backhand::{FilesystemCompressor, FilesystemWriter, NodeHeader};
use log::{debug, trace, warn};
use oci_spec::image::{ImageConfiguration, ImageManifest};
use std::borrow::Cow;
use std::fs::File;
use std::io::{BufWriter, ErrorKind, Read};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::pin::Pin;
use tokio::fs;
use tokio::io::AsyncRead;
use tokio_stream::StreamExt;
use tokio_tar::{Archive, Entry};
use uuid::Uuid;
use walkdir::WalkDir;

pub const IMAGE_SQUASHFS_VERSION: u64 = 2;

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
            let whiteouts = self.process_layer_whiteout(layer, image_dir).await?;
            debug!(
                "process layer digest={} whiteouts={:?}",
                &layer.digest, whiteouts
            );
            let mut archive = layer.archive().await?;
            let mut entries = archive.entries()?;
            while let Some(entry) = entries.next().await {
                let mut entry = entry?;
                let path = entry.path()?;
                let mut maybe_whiteout_path_str =
                    path.to_str().map(|x| x.to_string()).unwrap_or_default();
                if whiteouts.contains(&maybe_whiteout_path_str) {
                    continue;
                }
                maybe_whiteout_path_str.push('/');
                if whiteouts.contains(&maybe_whiteout_path_str) {
                    continue;
                }
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

    async fn process_layer_whiteout(
        &self,
        layer: &OciImageLayer,
        image_dir: &Path,
    ) -> Result<Vec<String>> {
        let mut whiteouts = Vec::new();
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
                let path = self
                    .process_whiteout_entry(&entry, name, layer, image_dir)
                    .await?;
                if let Some(path) = path {
                    whiteouts.push(path);
                }
            }
        }
        Ok(whiteouts)
    }

    async fn process_whiteout_entry(
        &self,
        entry: &Entry<Archive<Pin<Box<dyn AsyncRead + Send>>>>,
        name: &str,
        layer: &OciImageLayer,
        image_dir: &Path,
    ) -> Result<Option<String>> {
        let path = entry.path()?;
        let mut dst = self.check_safe_entry(path.clone(), image_dir)?;
        dst.pop();
        let mut path = path.to_path_buf();
        path.pop();

        let opaque = name == ".wh..wh..opq";

        if !opaque {
            let file = &name[4..];
            dst.push(file);
            path.push(file);
            self.check_safe_path(&dst, image_dir)?;
        }

        trace!("whiteout entry layer={} path={:?}", &layer.digest, path,);

        let whiteout = path
            .to_str()
            .ok_or(anyhow!("unable to convert path to string"))?
            .to_string();

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
                debug!(
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
            debug!(
                "whiteout entry missing locally layer={} path={:?} local={:?}",
                &layer.digest,
                entry.path()?,
                dst,
            );
        }
        Ok(if opaque { None } else { Some(whiteout) })
    }

    async fn process_write_entry(
        &self,
        entry: &mut Entry<Archive<Pin<Box<dyn AsyncRead + Send>>>>,
        layer: &OciImageLayer,
        image_dir: &Path,
    ) -> Result<()> {
        let uid = entry.header().uid()?;
        let gid = entry.header().gid()?;
        trace!(
            "unpack entry layer={} path={:?} type={:?} uid={} gid={}",
            &layer.digest,
            entry.path()?,
            entry.header().entry_type(),
            uid,
            gid,
        );
        entry.set_preserve_mtime(true);
        entry.set_preserve_permissions(true);
        entry.set_unpack_xattrs(true);
        if let Some(path) = entry.unpack_in(image_dir).await? {
            if !path.is_symlink() {
                std::os::unix::fs::chown(path, Some(uid as u32), Some(gid as u32))?;
            }
        }
        Ok(())
    }

    fn check_safe_entry(&self, path: Cow<Path>, image_dir: &Path) -> Result<PathBuf> {
        let mut dst = image_dir.to_path_buf();
        dst.push(path);
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
                writer.push_file(ConsumingFileReader::new(entry.path()), rel, header)?;
            } else if typ.is_block_device() {
                let device = metadata.dev();
                writer.push_block_device(device as u32, rel, header)?;
            } else if typ.is_char_device() {
                let device = metadata.dev();
                writer.push_char_device(device as u32, rel, header)?;
            } else if typ.is_fifo() {
                writer.push_fifo(rel, header)?;
            } else if typ.is_socket() {
                writer.push_socket(rel, header)?;
            } else {
                return Err(anyhow!("invalid file type"));
            }
        }

        let squash_file_path = squash_file
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert squashfs string"))?;

        let file = File::create(squash_file)?;
        let mut bufwrite = BufWriter::new(file);
        trace!("squash generate: {}", squash_file_path);
        writer.write(&mut bufwrite)?;
        std::fs::remove_dir_all(image_dir)?;
        Ok(())
    }
}

struct ConsumingFileReader {
    path: PathBuf,
    file: Option<File>,
}

impl ConsumingFileReader {
    fn new(path: &Path) -> ConsumingFileReader {
        ConsumingFileReader {
            path: path.to_path_buf(),
            file: None,
        }
    }
}

impl Read for ConsumingFileReader {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.file.is_none() {
            self.file = Some(File::open(&self.path)?);
        }
        let Some(ref mut file) = self.file else {
            return Err(std::io::Error::new(
                ErrorKind::NotFound,
                "file was not opened",
            ));
        };
        file.read(buf)
    }
}

impl Drop for ConsumingFileReader {
    fn drop(&mut self) {
        let file = self.file.take();
        drop(file);
        if let Err(error) = std::fs::remove_file(&self.path) {
            warn!("failed to delete consuming file {:?}: {}", self.path, error);
        }
    }
}
