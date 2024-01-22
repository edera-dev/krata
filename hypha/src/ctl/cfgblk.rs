use crate::error::Result;
use crate::image::ImageInfo;
use backhand::{FilesystemWriter, NodeHeader};
use std::fs;
use std::fs::File;
use std::path::PathBuf;
use uuid::Uuid;

pub struct ConfigBlock<'a> {
    pub image_info: &'a ImageInfo,
    pub config_bundle: Option<&'a str>,
    pub file: PathBuf,
    pub dir: PathBuf,
}

impl ConfigBlock<'_> {
    pub fn new<'a>(
        uuid: &Uuid,
        image_info: &'a ImageInfo,
        config_bundle: Option<&'a str>,
    ) -> Result<ConfigBlock<'a>> {
        let mut dir = std::env::temp_dir().clone();
        dir.push(format!("hypha-cfg-{}", uuid));
        fs::create_dir_all(&dir)?;
        let mut file = dir.clone();
        file.push("config.squashfs");
        Ok(ConfigBlock {
            image_info,
            config_bundle,
            file,
            dir,
        })
    }

    pub fn build(&self) -> Result<()> {
        let config_bundle_content = match self.config_bundle {
            None => None,
            Some(path) => Some(fs::read(path)?),
        };
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
        if let Some(config_bundle_content) = config_bundle_content.as_ref() {
            writer.push_file(
                config_bundle_content.as_slice(),
                "/bundle",
                NodeHeader {
                    permissions: 384,
                    uid: 0,
                    gid: 0,
                    mtime: 0,
                },
            )?;
        }
        let mut file = File::create(&self.file)?;
        writer.write(&mut file)?;
        Ok(())
    }
}
