use std::{
    fs::File,
    io::{BufWriter, ErrorKind, Read},
    os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{anyhow, Result};
use backhand::{compression::Compressor, FilesystemCompressor, FilesystemWriter, NodeHeader};
use log::{trace, warn};
use walkdir::WalkDir;

use crate::progress::{OciProgress, OciProgressContext, OciProgressPhase};

#[derive(Debug, Default, Clone, Copy)]
pub enum OciPackerFormat {
    #[default]
    Squashfs,
    Erofs,
}

#[derive(Debug, Clone, Copy)]
pub enum OciPackerBackendType {
    Backhand,
    MkSquashfs,
    MkfsErofs,
}

impl OciPackerFormat {
    pub fn id(&self) -> u8 {
        match self {
            OciPackerFormat::Squashfs => 0,
            OciPackerFormat::Erofs => 1,
        }
    }

    pub fn extension(&self) -> &str {
        match self {
            OciPackerFormat::Squashfs => "erofs",
            OciPackerFormat::Erofs => "erofs",
        }
    }

    pub fn detect_best_backend(&self) -> OciPackerBackendType {
        match self {
            OciPackerFormat::Squashfs => {
                let status = Command::new("mksquashfs")
                    .arg("-version")
                    .stdin(Stdio::null())
                    .stderr(Stdio::null())
                    .stdout(Stdio::null())
                    .status()
                    .ok();

                let Some(code) = status.and_then(|x| x.code()) else {
                    return OciPackerBackendType::Backhand;
                };

                if code == 0 {
                    OciPackerBackendType::MkSquashfs
                } else {
                    OciPackerBackendType::Backhand
                }
            }
            OciPackerFormat::Erofs => OciPackerBackendType::MkfsErofs,
        }
    }
}

impl OciPackerBackendType {
    pub fn format(&self) -> OciPackerFormat {
        match self {
            OciPackerBackendType::Backhand => OciPackerFormat::Squashfs,
            OciPackerBackendType::MkSquashfs => OciPackerFormat::Squashfs,
            OciPackerBackendType::MkfsErofs => OciPackerFormat::Erofs,
        }
    }

    pub fn create(&self) -> Box<dyn OciPackerBackend> {
        match self {
            OciPackerBackendType::Backhand => {
                Box::new(OciPackerBackhand {}) as Box<dyn OciPackerBackend>
            }
            OciPackerBackendType::MkSquashfs => {
                Box::new(OciPackerMkSquashfs {}) as Box<dyn OciPackerBackend>
            }
            OciPackerBackendType::MkfsErofs => {
                Box::new(OciPackerMkfsErofs {}) as Box<dyn OciPackerBackend>
            }
        }
    }
}

pub trait OciPackerBackend {
    fn pack(
        &self,
        progress: &mut OciProgress,
        progress_context: &OciProgressContext,
        directory: &Path,
        file: &Path,
    ) -> Result<()>;
}

pub struct OciPackerBackhand {}

impl OciPackerBackend for OciPackerBackhand {
    fn pack(
        &self,
        progress: &mut OciProgress,
        progress_context: &OciProgressContext,
        directory: &Path,
        file: &Path,
    ) -> Result<()> {
        progress.phase = OciProgressPhase::Packing;
        progress.total = 1;
        progress.value = 0;
        progress_context.update(progress);
        let mut writer = FilesystemWriter::default();
        writer.set_compressor(FilesystemCompressor::new(Compressor::Gzip, None)?);
        let walk = WalkDir::new(directory).follow_links(false);
        for entry in walk {
            let entry = entry?;
            let rel = entry
                .path()
                .strip_prefix(directory)?
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

        progress.phase = OciProgressPhase::Packing;
        progress.value = 1;
        progress_context.update(progress);

        let squash_file_path = file
            .to_str()
            .ok_or_else(|| anyhow!("failed to convert squashfs string"))?;

        let file = File::create(file)?;
        let mut bufwrite = BufWriter::new(file);
        trace!("squash generate: {}", squash_file_path);
        writer.write(&mut bufwrite)?;
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

pub struct OciPackerMkSquashfs {}

impl OciPackerBackend for OciPackerMkSquashfs {
    fn pack(
        &self,
        progress: &mut OciProgress,
        progress_context: &OciProgressContext,
        directory: &Path,
        file: &Path,
    ) -> Result<()> {
        progress.phase = OciProgressPhase::Packing;
        progress.total = 1;
        progress.value = 0;
        progress_context.update(progress);
        let mut child = Command::new("mksquashfs")
            .arg(directory)
            .arg(file)
            .arg("-comp")
            .arg("gzip")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?;
        let status = child.wait()?;
        if !status.success() {
            Err(anyhow!(
                "mksquashfs failed with exit code: {}",
                status.code().unwrap()
            ))
        } else {
            progress.phase = OciProgressPhase::Packing;
            progress.total = 1;
            progress.value = 1;
            progress_context.update(progress);
            Ok(())
        }
    }
}

pub struct OciPackerMkfsErofs {}

impl OciPackerBackend for OciPackerMkfsErofs {
    fn pack(
        &self,
        progress: &mut OciProgress,
        progress_context: &OciProgressContext,
        directory: &Path,
        file: &Path,
    ) -> Result<()> {
        progress.phase = OciProgressPhase::Packing;
        progress.total = 1;
        progress.value = 0;
        progress_context.update(progress);
        let mut child = Command::new("mkfs.erofs")
            .arg("-L")
            .arg("root")
            .arg(file)
            .arg(directory)
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .spawn()?;
        let status = child.wait()?;
        if !status.success() {
            Err(anyhow!(
                "mkfs.erofs failed with exit code: {}",
                status.code().unwrap()
            ))
        } else {
            progress.phase = OciProgressPhase::Packing;
            progress.total = 1;
            progress.value = 1;
            progress_context.update(progress);
            Ok(())
        }
    }
}
