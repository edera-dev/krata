use crate::cache::ImageCache;
use crate::fetch::{OciImageDownloader, OciImageLayer};
use crate::name::ImageName;
use crate::packer::OciPackerFormat;
use crate::progress::{OciProgress, OciProgressContext, OciProgressPhase};
use crate::registry::OciRegistryPlatform;
use anyhow::{anyhow, Result};
use indexmap::IndexMap;
use log::{debug, trace};
use oci_spec::image::{ImageConfiguration, ImageManifest};
use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use tokio::fs;
use tokio::io::AsyncRead;
use tokio_stream::StreamExt;
use tokio_tar::{Archive, Entry};
use uuid::Uuid;

pub const IMAGE_PACKER_VERSION: u64 = 2;

pub struct ImageInfo {
    pub image: PathBuf,
    pub manifest: ImageManifest,
    pub config: ImageConfiguration,
}

impl ImageInfo {
    pub fn new(
        image: PathBuf,
        manifest: ImageManifest,
        config: ImageConfiguration,
    ) -> Result<ImageInfo> {
        Ok(ImageInfo {
            image,
            manifest,
            config,
        })
    }
}

pub struct OciImageCompiler<'a> {
    cache: &'a ImageCache,
    seed: Option<PathBuf>,
    progress: OciProgressContext,
}

impl OciImageCompiler<'_> {
    pub fn new(
        cache: &ImageCache,
        seed: Option<PathBuf>,
        progress: OciProgressContext,
    ) -> Result<OciImageCompiler> {
        Ok(OciImageCompiler {
            cache,
            seed,
            progress,
        })
    }

    pub async fn compile(
        &self,
        id: &str,
        image: &ImageName,
        format: OciPackerFormat,
    ) -> Result<ImageInfo> {
        debug!("compile image={image} format={:?}", format);
        let mut tmp_dir = std::env::temp_dir().clone();
        tmp_dir.push(format!("krata-compile-{}", Uuid::new_v4()));

        let mut image_dir = tmp_dir.clone();
        image_dir.push("image");
        fs::create_dir_all(&image_dir).await?;

        let mut layer_dir = tmp_dir.clone();
        layer_dir.push("layer");
        fs::create_dir_all(&layer_dir).await?;

        let mut packed_file = tmp_dir.clone();
        packed_file.push("image.packed");

        let _guard = scopeguard::guard(tmp_dir.to_path_buf(), |delete| {
            tokio::task::spawn(async move {
                let _ = fs::remove_dir_all(delete).await;
            });
        });
        let info = self
            .download_and_compile(id, image, &layer_dir, &image_dir, &packed_file, format)
            .await?;
        Ok(info)
    }

    async fn download_and_compile(
        &self,
        id: &str,
        image: &ImageName,
        layer_dir: &Path,
        image_dir: &Path,
        packed_file: &Path,
        format: OciPackerFormat,
    ) -> Result<ImageInfo> {
        let mut progress = OciProgress {
            id: id.to_string(),
            phase: OciProgressPhase::Resolving,
            layers: IndexMap::new(),
            value: 0,
            total: 0,
        };
        self.progress.update(&progress);
        let downloader = OciImageDownloader::new(
            self.seed.clone(),
            layer_dir.to_path_buf(),
            OciRegistryPlatform::current(),
            self.progress.clone(),
        );
        let resolved = downloader.resolve(image.clone()).await?;
        let cache_key = format!(
            "manifest={}:version={}:format={}\n",
            resolved.digest,
            IMAGE_PACKER_VERSION,
            format.id(),
        );
        let cache_digest = sha256::digest(cache_key);

        if let Some(cached) = self.cache.recall(&cache_digest, format).await? {
            return Ok(cached);
        }

        progress.phase = OciProgressPhase::Resolved;
        for layer in resolved.manifest.layers() {
            progress.add_layer(layer.digest());
        }
        self.progress.update(&progress);

        let local = downloader.download(resolved, &mut progress).await?;
        for layer in &local.layers {
            debug!(
                "process layer digest={} compression={:?}",
                &layer.digest, layer.compression,
            );
            progress.extracting_layer(&layer.digest, 0, 1);
            self.progress.update(&progress);
            let (whiteouts, count) = self.process_layer_whiteout(layer, image_dir).await?;
            progress.extracting_layer(&layer.digest, 0, count);
            self.progress.update(&progress);
            debug!(
                "process layer digest={} whiteouts={:?}",
                &layer.digest, whiteouts
            );
            let mut archive = layer.archive().await?;
            let mut entries = archive.entries()?;
            let mut completed = 0;
            while let Some(entry) = entries.next().await {
                let mut entry = entry?;
                let path = entry.path()?;
                let mut maybe_whiteout_path_str =
                    path.to_str().map(|x| x.to_string()).unwrap_or_default();
                if (completed % 10) == 0 {
                    progress.extracting_layer(&layer.digest, completed, count);
                }
                completed += 1;
                self.progress.update(&progress);
                if whiteouts.contains(&maybe_whiteout_path_str) {
                    continue;
                }
                maybe_whiteout_path_str.push('/');
                if whiteouts.contains(&maybe_whiteout_path_str) {
                    continue;
                }
                let Some(name) = path.file_name() else {
                    continue;
                };
                let Some(name) = name.to_str() else {
                    continue;
                };

                if name.starts_with(".wh.") {
                    continue;
                } else {
                    self.process_write_entry(&mut entry, layer, image_dir)
                        .await?;
                }
            }
            progress.extracted_layer(&layer.digest);
            self.progress.update(&progress);
        }

        for layer in &local.layers {
            if layer.path.exists() {
                fs::remove_file(&layer.path).await?;
            }
        }

        let image_dir_pack = image_dir.to_path_buf();
        let packed_file_pack = packed_file.to_path_buf();
        let progress_pack = progress.clone();
        let progress_context = self.progress.clone();
        let format_pack = format;
        progress = tokio::task::spawn_blocking(move || {
            OciImageCompiler::pack(
                format_pack,
                &image_dir_pack,
                &packed_file_pack,
                progress_pack,
                progress_context,
            )
        })
        .await??;

        let info = ImageInfo::new(
            packed_file.to_path_buf(),
            local.image.manifest,
            local.config,
        )?;
        let info = self.cache.store(&cache_digest, &info, format).await?;
        progress.phase = OciProgressPhase::Complete;
        progress.value = 0;
        progress.total = 0;
        self.progress.update(&progress);
        Ok(info)
    }

    async fn process_layer_whiteout(
        &self,
        layer: &OciImageLayer,
        image_dir: &Path,
    ) -> Result<(Vec<String>, usize)> {
        let mut whiteouts = Vec::new();
        let mut archive = layer.archive().await?;
        let mut entries = archive.entries()?;
        let mut count = 0usize;
        while let Some(entry) = entries.next().await {
            let entry = entry?;
            count += 1;
            let path = entry.path()?;
            let Some(name) = path.file_name() else {
                continue;
            };
            let Some(name) = name.to_str() else {
                continue;
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
        Ok((whiteouts, count))
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

    fn pack(
        format: OciPackerFormat,
        image_dir: &Path,
        packed_file: &Path,
        mut progress: OciProgress,
        progress_context: OciProgressContext,
    ) -> Result<OciProgress> {
        let backend = format.detect_best_backend();
        let backend = backend.create();
        backend.pack(&mut progress, &progress_context, image_dir, packed_file)?;
        std::fs::remove_dir_all(image_dir)?;
        progress.phase = OciProgressPhase::Packing;
        progress.value = progress.total;
        progress_context.update(&progress);
        Ok(progress)
    }
}
