use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};
use tokio::{
    fs::File,
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
};
use tokio_tar::{Builder, Entry, EntryType, Header};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum VfsNodeType {
    Directory,
    RegularFile,
    Symlink,
    Hardlink,
    Fifo,
    CharDevice,
    BlockDevice,
}

#[derive(Clone, Debug)]
pub struct VfsNode {
    pub name: String,
    pub size: u64,
    pub children: Vec<VfsNode>,
    pub typ: VfsNodeType,
    pub uid: u64,
    pub gid: u64,
    pub link_name: Option<String>,
    pub mode: u32,
    pub mtime: u64,
    pub dev_major: Option<u32>,
    pub dev_minor: Option<u32>,
    pub disk_path: Option<PathBuf>,
}

impl VfsNode {
    pub fn from<X: AsyncRead + Unpin>(entry: &Entry<X>) -> Result<VfsNode> {
        let header = entry.header();
        let name = entry
            .path()?
            .file_name()
            .ok_or(anyhow!("unable to get file name for entry"))?
            .to_string_lossy()
            .to_string();
        let typ = header.entry_type();
        let vtype = if typ.is_symlink() {
            VfsNodeType::Symlink
        } else if typ.is_hard_link() {
            VfsNodeType::Hardlink
        } else if typ.is_dir() {
            VfsNodeType::Directory
        } else if typ.is_fifo() {
            VfsNodeType::Fifo
        } else if typ.is_block_special() {
            VfsNodeType::BlockDevice
        } else if typ.is_character_special() {
            VfsNodeType::CharDevice
        } else if typ.is_file() {
            VfsNodeType::RegularFile
        } else {
            return Err(anyhow!("unable to determine vfs type for entry"));
        };

        Ok(VfsNode {
            name,
            size: header.size()?,
            children: vec![],
            typ: vtype,
            uid: header.uid()?,
            gid: header.gid()?,
            link_name: header.link_name()?.map(|x| x.to_string_lossy().to_string()),
            mode: header.mode()?,
            mtime: header.mtime()?,
            dev_major: header.device_major()?,
            dev_minor: header.device_minor()?,
            disk_path: None,
        })
    }

    pub fn lookup(&self, path: &Path) -> Option<&VfsNode> {
        let mut node = self;
        for part in path {
            node = node
                .children
                .iter()
                .find(|child| child.name == part.to_string_lossy())?;
        }
        Some(node)
    }

    pub fn lookup_mut(&mut self, path: &Path) -> Option<&mut VfsNode> {
        let mut node = self;
        for part in path {
            node = node
                .children
                .iter_mut()
                .find(|child| child.name == part.to_string_lossy())?;
        }
        Some(node)
    }

    pub fn remove(&mut self, path: &Path) -> Option<(&mut VfsNode, VfsNode)> {
        let parent = path.parent()?;
        let node = self.lookup_mut(parent)?;
        let file_name = path.file_name()?;
        let file_name = file_name.to_string_lossy();
        let position = node
            .children
            .iter()
            .position(|child| file_name == child.name)?;
        let removed = node.children.remove(position);
        Some((node, removed))
    }

    pub fn create_tar_header(&self) -> Result<Header> {
        let mut header = Header::new_ustar();
        header.set_entry_type(match self.typ {
            VfsNodeType::Directory => EntryType::Directory,
            VfsNodeType::CharDevice => EntryType::Char,
            VfsNodeType::BlockDevice => EntryType::Block,
            VfsNodeType::Fifo => EntryType::Fifo,
            VfsNodeType::Hardlink => EntryType::Link,
            VfsNodeType::Symlink => EntryType::Symlink,
            VfsNodeType::RegularFile => EntryType::Regular,
        });
        header.set_uid(self.uid);
        header.set_gid(self.gid);

        if let Some(device_major) = self.dev_major {
            header.set_device_major(device_major)?;
        }

        if let Some(device_minor) = self.dev_minor {
            header.set_device_minor(device_minor)?;
        }
        header.set_mtime(self.mtime);
        header.set_mode(self.mode);

        if let Some(link_name) = self.link_name.as_ref() {
            header.set_link_name(&PathBuf::from(link_name))?;
        }
        header.set_size(self.size);
        Ok(header)
    }

    pub async fn write_to_tar<W: AsyncWrite + Unpin + Send>(
        &self,
        path: &Path,
        builder: &mut Builder<W>,
    ) -> Result<()> {
        let mut header = self.create_tar_header()?;
        header.set_path(path)?;
        header.set_cksum();
        if let Some(disk_path) = self.disk_path.as_ref() {
            builder
                .append(&header, File::open(disk_path).await?)
                .await?;
        } else {
            builder.append(&header, &[] as &[u8]).await?;
        }
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct VfsTree {
    pub root: VfsNode,
}

impl Default for VfsTree {
    fn default() -> Self {
        Self::new()
    }
}

impl VfsTree {
    pub fn new() -> VfsTree {
        VfsTree {
            root: VfsNode {
                name: "".to_string(),
                size: 0,
                children: vec![],
                typ: VfsNodeType::Directory,
                uid: 0,
                gid: 0,
                link_name: None,
                mode: 0,
                mtime: 0,
                dev_major: None,
                dev_minor: None,
                disk_path: None,
            },
        }
    }

    pub fn insert_tar_entry<X: AsyncRead + Unpin>(&mut self, entry: &Entry<X>) -> Result<&VfsNode> {
        let mut meta = VfsNode::from(entry)?;
        let path = entry.path()?.to_path_buf();
        let parent = if let Some(parent) = path.parent() {
            self.root.lookup_mut(parent)
        } else {
            Some(&mut self.root)
        };

        let Some(parent) = parent else {
            return Err(anyhow!("unable to find parent of entry"));
        };

        let position = parent
            .children
            .iter()
            .position(|child| meta.name == child.name);

        if let Some(position) = position {
            let old = parent.children.remove(position);
            if meta.typ == VfsNodeType::Directory {
                meta.children = old.children;
            }
        }
        parent.children.push(meta.clone());
        let Some(reference) = parent.children.iter().find(|child| child.name == meta.name) else {
            return Err(anyhow!("unable to find inserted child in vfs"));
        };
        Ok(reference)
    }

    pub fn set_disk_path(&mut self, path: &Path, disk_path: &Path) -> Result<()> {
        let Some(node) = self.root.lookup_mut(path) else {
            return Err(anyhow!(
                "unable to find node {:?} to set disk path to",
                path
            ));
        };
        node.disk_path = Some(disk_path.to_path_buf());
        Ok(())
    }

    pub async fn write_to_tar<W: AsyncWrite + Unpin + Send + 'static>(
        &self,
        write: W,
    ) -> Result<()> {
        let mut builder = Builder::new(write);
        let mut queue = vec![(PathBuf::from(""), &self.root)];

        while !queue.is_empty() {
            let (mut path, node) = queue.remove(0);
            if !node.name.is_empty() {
                path.push(&node.name);
            }
            if path.components().count() != 0 {
                node.write_to_tar(&path, &mut builder).await?;
            }
            for child in &node.children {
                queue.push((path.clone(), child));
            }
        }

        let mut write = builder.into_inner().await?;
        write.flush().await?;
        drop(write);
        Ok(())
    }
}
