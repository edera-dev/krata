use crate::error::Result;
use crate::image::ImageInfo;
use backhand::{FilesystemWriter, NodeHeader};
use std::fs;
use std::fs::File;
use std::path::PathBuf;
use uuid::Uuid;

pub struct ConfigBlock<'a> {
    pub image_info: &'a ImageInfo,
    pub file: PathBuf,
}

impl ConfigBlock<'_> {
    pub fn new<'a>(uuid: &Uuid, image_info: &'a ImageInfo) -> Result<ConfigBlock<'a>> {
        let mut file = std::env::temp_dir().clone();
        file.push(format!("hypha-cfg-{}", uuid));
        fs::create_dir_all(&file)?;
        file.push("config.squashfs");
        Ok(ConfigBlock { image_info, file })
    }

    pub fn build(&self) -> Result<()> {
        let manifest = self.image_info.config.to_string()?;
        let mut writer = FilesystemWriter::default();
        writer.push_dir(
            "/image",
            NodeHeader {
                permissions: 384,
                uid: 0,
                gid: 0,
                mtime: 0,
            },
        )?;
        writer.push_file(
            manifest.as_bytes(),
            "/image/config.json",
            NodeHeader {
                permissions: 384,
                uid: 0,
                gid: 0,
                mtime: 0,
            },
        )?;
        let mut file = File::create(&self.file)?;
        writer.write(&mut file)?;
        Ok(())
    }
}
