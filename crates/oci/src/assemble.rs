use crate::fetch::{OciImageFetcher, OciImageLayer, OciResolvedImage};
use crate::progress::OciBoundProgress;
use crate::schema::OciSchema;
use crate::vfs::{VfsNode, VfsTree};
use anyhow::{anyhow, Result};
use log::{debug, trace, warn};
use oci_spec::image::{ImageConfiguration, ImageManifest};

use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::fs;
use tokio::io::AsyncRead;
use tokio_stream::StreamExt;
use tokio_tar::{Archive, Entry};
use uuid::Uuid;

pub struct OciImageAssembled {
    pub digest: String,
    pub manifest: OciSchema<ImageManifest>,
    pub config: OciSchema<ImageConfiguration>,
    pub vfs: Arc<VfsTree>,
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
    resolved: Option<OciResolvedImage>,
    progress: OciBoundProgress,
    work_dir: PathBuf,
    disk_dir: PathBuf,
    tmp_dir: Option<PathBuf>,
    success: AtomicBool,
}

impl OciImageAssembler {
    pub async fn new(
        downloader: OciImageFetcher,
        resolved: OciResolvedImage,
        progress: OciBoundProgress,
        work_dir: Option<PathBuf>,
        disk_dir: Option<PathBuf>,
    ) -> Result<OciImageAssembler> {
        let tmp_dir = if work_dir.is_none() || disk_dir.is_none() {
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

        let target_dir = if let Some(target_dir) = disk_dir {
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
            resolved: Some(resolved),
            progress,
            work_dir,
            disk_dir: target_dir,
            tmp_dir,
            success: AtomicBool::new(false),
        })
    }

    pub async fn assemble(self) -> Result<OciImageAssembled> {
        debug!("assemble");
        let mut layer_dir = self.work_dir.clone();
        layer_dir.push("layer");
        fs::create_dir_all(&layer_dir).await?;
        self.assemble_with(&layer_dir).await
    }

    async fn assemble_with(mut self, layer_dir: &Path) -> Result<OciImageAssembled> {
        let Some(ref resolved) = self.resolved else {
            return Err(anyhow!("resolved image was not available when expected"));
        };
        let local = self.downloader.download(resolved, layer_dir).await?;
        let mut vfs = VfsTree::new();
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
            debug!("process layer digest={}", &layer.digest,);
            let mut archive = layer.archive().await?;
            let mut entries = archive.entries()?;
            while let Some(entry) = entries.next().await {
                let mut entry = entry?;
                let path = entry.path()?;
                let Some(name) = path.file_name() else {
                    continue;
                };
                let Some(name) = name.to_str() else {
                    continue;
                };
                if name.starts_with(".wh.") {
                    self.process_whiteout_entry(&mut vfs, &entry, name, layer)
                        .await?;
                } else {
                    vfs.insert_tar_entry(&entry)?;
                    self.process_write_entry(&mut vfs, &mut entry, layer)
                        .await?;
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

        let Some(resolved) = self.resolved.take() else {
            return Err(anyhow!("resolved image was not available when expected"));
        };

        let assembled = OciImageAssembled {
            vfs: Arc::new(vfs),
            digest: resolved.digest,
            manifest: resolved.manifest,
            config: local.config,
            tmp_dir: self.tmp_dir.clone(),
        };
        self.success.store(true, Ordering::Release);
        Ok(assembled)
    }

    async fn process_whiteout_entry(
        &self,
        vfs: &mut VfsTree,
        entry: &Entry<Archive<Pin<Box<dyn AsyncRead + Send>>>>,
        name: &str,
        layer: &OciImageLayer,
    ) -> Result<()> {
        let path = entry.path()?;
        let mut path = path.to_path_buf();
        path.pop();

        let opaque = name == ".wh..wh..opq";

        if !opaque {
            let file = &name[4..];
            path.push(file);
        }

        trace!(
            "whiteout entry {:?} layer={} path={:?}",
            entry.path()?,
            &layer.digest,
            path
        );

        let result = vfs.root.remove(&path);
        if let Some((parent, mut removed)) = result {
            delete_disk_paths(&removed).await?;
            if opaque {
                removed.children.clear();
                parent.children.push(removed);
            }
        } else {
            warn!(
                "whiteout entry layer={} path={:?} did not exist",
                &layer.digest, path
            );
        }
        Ok(())
    }

    async fn process_write_entry(
        &self,
        vfs: &mut VfsTree,
        entry: &mut Entry<Archive<Pin<Box<dyn AsyncRead + Send>>>>,
        layer: &OciImageLayer,
    ) -> Result<()> {
        if !entry.header().entry_type().is_file() {
            return Ok(());
        }
        trace!(
            "unpack entry layer={} path={:?} type={:?}",
            &layer.digest,
            entry.path()?,
            entry.header().entry_type(),
        );
        entry.set_preserve_permissions(false);
        entry.set_unpack_xattrs(false);
        entry.set_preserve_mtime(false);
        let path = entry
            .unpack_in(&self.disk_dir)
            .await?
            .ok_or(anyhow!("unpack did not return a path"))?;
        vfs.set_disk_path(&entry.path()?, &path)?;
        Ok(())
    }
}

impl Drop for OciImageAssembler {
    fn drop(&mut self) {
        if !self.success.load(Ordering::Acquire) {
            if let Some(tmp_dir) = self.tmp_dir.clone() {
                tokio::task::spawn(async move {
                    let _ = fs::remove_dir_all(tmp_dir).await;
                });
            }
        }
    }
}

async fn delete_disk_paths(node: &VfsNode) -> Result<()> {
    let mut queue = vec![node];
    while !queue.is_empty() {
        let node = queue.remove(0);
        if let Some(ref disk_path) = node.disk_path {
            if !disk_path.exists() {
                warn!("disk path {:?} does not exist", disk_path);
            }
            fs::remove_file(disk_path).await?;
        }
        let children = node.children.iter().collect::<Vec<_>>();
        queue.extend_from_slice(&children);
    }
    Ok(())
}
