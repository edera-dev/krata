use crate::fetch::{OciImageFetcher, OciImageLayer, OciResolvedImage};
use crate::progress::OciBoundProgress;
use anyhow::{anyhow, Result};
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

pub struct OciImageAssembled {
    pub digest: String,
    pub path: PathBuf,
    pub manifest: ImageManifest,
    pub config: ImageConfiguration,
    pub tmp_dir: Option<PathBuf>,
}

impl Drop for OciImageAssembled {
    fn drop(&mut self) {
        if let Some(tmp) = self.tmp_dir.clone() {
            tokio::task::spawn(async move {
                let _ = fs::remove_dir_all(&tmp).await;
            });
        }
    }
}

pub struct OciImageAssembler {
    downloader: OciImageFetcher,
    resolved: OciResolvedImage,
    progress: OciBoundProgress,
    work_dir: PathBuf,
    target_dir: PathBuf,
    tmp_dir: Option<PathBuf>,
}

impl OciImageAssembler {
    pub async fn new(
        downloader: OciImageFetcher,
        resolved: OciResolvedImage,
        progress: OciBoundProgress,
        work_dir: Option<PathBuf>,
        target_dir: Option<PathBuf>,
    ) -> Result<OciImageAssembler> {
        let tmp_dir = if work_dir.is_none() || target_dir.is_none() {
            let mut tmp_dir = std::env::temp_dir().clone();
            tmp_dir.push(format!("oci-assemble-{}", Uuid::new_v4()));
            Some(tmp_dir)
        } else {
            None
        };

        let work_dir = if let Some(work_dir) = work_dir {
            work_dir
        } else {
            let mut tmp_dir = tmp_dir
                .clone()
                .ok_or(anyhow!("tmp_dir was not created when expected"))?;
            tmp_dir.push("work");
            tmp_dir
        };

        let target_dir = if let Some(target_dir) = target_dir {
            target_dir
        } else {
            let mut tmp_dir = tmp_dir
                .clone()
                .ok_or(anyhow!("tmp_dir was not created when expected"))?;
            tmp_dir.push("image");
            tmp_dir
        };

        fs::create_dir_all(&work_dir).await?;
        fs::create_dir_all(&target_dir).await?;

        Ok(OciImageAssembler {
            downloader,
            resolved,
            progress,
            work_dir,
            target_dir,
            tmp_dir,
        })
    }

    pub async fn assemble(self) -> Result<OciImageAssembled> {
        debug!("assemble");
        let mut layer_dir = self.work_dir.clone();
        layer_dir.push("layer");
        fs::create_dir_all(&layer_dir).await?;
        self.assemble_with(&layer_dir).await
    }

    async fn assemble_with(self, layer_dir: &Path) -> Result<OciImageAssembled> {
        let local = self
            .downloader
            .download(self.resolved.clone(), layer_dir)
            .await?;
        for layer in &local.layers {
            debug!(
                "process layer digest={} compression={:?}",
                &layer.digest, layer.compression,
            );
            self.progress
                .update(|progress| {
                    progress.extracting_layer(&layer.digest, 0, 1);
                })
                .await;
            let (whiteouts, count) = self.process_layer_whiteout(layer).await?;
            self.progress
                .update(|progress| {
                    progress.extracting_layer(&layer.digest, 0, count);
                })
                .await;
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
                    self.progress
                        .update(|progress| {
                            progress.extracting_layer(&layer.digest, completed, count);
                        })
                        .await;
                }
                completed += 1;
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
                    self.process_write_entry(&mut entry, layer).await?;
                }
            }
            self.progress
                .update(|progress| {
                    progress.extracted_layer(&layer.digest);
                })
                .await;
        }

        for layer in &local.layers {
            if layer.path.exists() {
                fs::remove_file(&layer.path).await?;
            }
        }
        Ok(OciImageAssembled {
            digest: self.resolved.digest,
            path: self.target_dir,
            manifest: self.resolved.manifest,
            config: local.config,
            tmp_dir: self.tmp_dir,
        })
    }

    async fn process_layer_whiteout(&self, layer: &OciImageLayer) -> Result<(Vec<String>, usize)> {
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
                let path = self.process_whiteout_entry(&entry, name, layer).await?;
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
    ) -> Result<Option<String>> {
        let path = entry.path()?;
        let mut dst = self.check_safe_entry(path.clone())?;
        dst.pop();
        let mut path = path.to_path_buf();
        path.pop();

        let opaque = name == ".wh..wh..opq";

        if !opaque {
            let file = &name[4..];
            dst.push(file);
            path.push(file);
            self.check_safe_path(&dst)?;
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
        if let Some(path) = entry.unpack_in(&self.target_dir).await? {
            if !path.is_symlink() {
                std::os::unix::fs::chown(path, Some(uid as u32), Some(gid as u32))?;
            }
        }
        Ok(())
    }

    fn check_safe_entry(&self, path: Cow<Path>) -> Result<PathBuf> {
        let mut dst = self.target_dir.to_path_buf();
        dst.push(path);
        if let Some(name) = dst.file_name() {
            if let Some(name) = name.to_str() {
                if name.starts_with(".wh.") {
                    let copy = dst.clone();
                    dst.pop();
                    self.check_safe_path(&dst)?;
                    return Ok(copy);
                }
            }
        }
        self.check_safe_path(&dst)?;
        Ok(dst)
    }

    fn check_safe_path(&self, dst: &Path) -> Result<()> {
        let resolved = path_clean::clean(dst);
        if !resolved.starts_with(&self.target_dir) {
            return Err(anyhow!("layer attempts to work outside image dir"));
        }
        Ok(())
    }
}
