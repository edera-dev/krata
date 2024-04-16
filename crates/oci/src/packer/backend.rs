use std::{path::Path, process::Stdio, sync::Arc};

use super::OciPackedFormat;
use crate::{
    progress::{OciBoundProgress, OciProgressPhase},
    vfs::VfsTree,
};
use anyhow::{anyhow, Result};
use log::warn;
use tokio::{
    fs::File,
    pin,
    process::{Child, Command},
    select,
};

#[derive(Debug, Clone, Copy)]
pub enum OciPackerBackendType {
    MkSquashfs,
    MkfsErofs,
    Tar,
}

impl OciPackerBackendType {
    pub fn format(&self) -> OciPackedFormat {
        match self {
            OciPackerBackendType::MkSquashfs => OciPackedFormat::Squashfs,
            OciPackerBackendType::MkfsErofs => OciPackedFormat::Erofs,
            OciPackerBackendType::Tar => OciPackedFormat::Tar,
        }
    }

    pub fn create(&self) -> Box<dyn OciPackerBackend> {
        match self {
            OciPackerBackendType::MkSquashfs => {
                Box::new(OciPackerMkSquashfs {}) as Box<dyn OciPackerBackend>
            }
            OciPackerBackendType::MkfsErofs => {
                Box::new(OciPackerMkfsErofs {}) as Box<dyn OciPackerBackend>
            }
            OciPackerBackendType::Tar => Box::new(OciPackerTar {}) as Box<dyn OciPackerBackend>,
        }
    }
}

#[async_trait::async_trait]
pub trait OciPackerBackend: Send + Sync {
    async fn pack(&self, progress: OciBoundProgress, vfs: Arc<VfsTree>, file: &Path) -> Result<()>;
}

pub struct OciPackerMkSquashfs {}

#[async_trait::async_trait]
impl OciPackerBackend for OciPackerMkSquashfs {
    async fn pack(&self, progress: OciBoundProgress, vfs: Arc<VfsTree>, file: &Path) -> Result<()> {
        progress
            .update(|progress| {
                progress.phase = OciProgressPhase::Packing;
                progress.total = 1;
                progress.value = 0;
            })
            .await;

        let child = Command::new("mksquashfs")
            .arg("-")
            .arg(file)
            .arg("-comp")
            .arg("gzip")
            .arg("-tar")
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?;
        let mut child = ChildProcessKillGuard(child);
        let stdin = child
            .0
            .stdin
            .take()
            .ok_or(anyhow!("unable to acquire stdin stream"))?;
        let mut writer = Some(tokio::task::spawn(async move {
            if let Err(error) = vfs.write_to_tar(stdin).await {
                warn!("failed to write tar: {}", error);
                return Err(error);
            }
            Ok(())
        }));
        let wait = child.0.wait();
        pin!(wait);
        let status_result = loop {
            if let Some(inner) = writer.as_mut() {
                select! {
                    x = inner => {
                        writer = None;
                        match x {
                            Ok(_) => {},
                            Err(error) => {
                                return Err(error.into());
                            }
                        }
                    },
                    status = &mut wait => {
                        break status;
                    }
                };
            } else {
                select! {
                    status = &mut wait => {
                        break status;
                    }
                };
            }
        };
        if let Some(writer) = writer {
            writer.await??;
        }
        let status = status_result?;
        if !status.success() {
            Err(anyhow!(
                "mksquashfs failed with exit code: {}",
                status.code().unwrap()
            ))
        } else {
            progress
                .update(|progress| {
                    progress.phase = OciProgressPhase::Packing;
                    progress.total = 1;
                    progress.value = 1;
                })
                .await;
            Ok(())
        }
    }
}

pub struct OciPackerMkfsErofs {}

#[async_trait::async_trait]
impl OciPackerBackend for OciPackerMkfsErofs {
    async fn pack(&self, progress: OciBoundProgress, vfs: Arc<VfsTree>, path: &Path) -> Result<()> {
        progress
            .update(|progress| {
                progress.phase = OciProgressPhase::Packing;
                progress.total = 1;
                progress.value = 0;
            })
            .await;

        let child = Command::new("mkfs.erofs")
            .arg("-L")
            .arg("root")
            .arg("--tar=-")
            .arg(path)
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?;
        let mut child = ChildProcessKillGuard(child);
        let stdin = child
            .0
            .stdin
            .take()
            .ok_or(anyhow!("unable to acquire stdin stream"))?;
        let mut writer = Some(tokio::task::spawn(
            async move { vfs.write_to_tar(stdin).await },
        ));
        let wait = child.0.wait();
        pin!(wait);
        let status_result = loop {
            if let Some(inner) = writer.as_mut() {
                select! {
                    x = inner => {
                        match x {
                            Ok(_) => {
                                writer = None;
                            },
                            Err(error) => {
                                return Err(error.into());
                            }
                        }
                    },
                    status = &mut wait => {
                        break status;
                    }
                };
            } else {
                select! {
                    status = &mut wait => {
                        break status;
                    }
                };
            }
        };
        if let Some(writer) = writer {
            writer.await??;
        }
        let status = status_result?;
        if !status.success() {
            Err(anyhow!(
                "mkfs.erofs failed with exit code: {}",
                status.code().unwrap()
            ))
        } else {
            progress
                .update(|progress| {
                    progress.phase = OciProgressPhase::Packing;
                    progress.total = 1;
                    progress.value = 1;
                })
                .await;
            Ok(())
        }
    }
}

pub struct OciPackerTar {}

#[async_trait::async_trait]
impl OciPackerBackend for OciPackerTar {
    async fn pack(&self, progress: OciBoundProgress, vfs: Arc<VfsTree>, file: &Path) -> Result<()> {
        progress
            .update(|progress| {
                progress.phase = OciProgressPhase::Packing;
                progress.total = 1;
                progress.value = 0;
            })
            .await;

        let file = File::create(file).await?;
        vfs.write_to_tar(file).await?;

        progress
            .update(|progress| {
                progress.phase = OciProgressPhase::Packing;
                progress.total = 1;
                progress.value = 1;
            })
            .await;
        Ok(())
    }
}

struct ChildProcessKillGuard(Child);

impl Drop for ChildProcessKillGuard {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}
