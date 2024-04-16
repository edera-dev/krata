use anyhow::Result;
use backhand::{FilesystemWriter, NodeHeader};
use krata::launchcfg::LaunchInfo;
use krataoci::packer::OciPackedImage;
use log::trace;
use std::fs;
use std::fs::File;
use std::path::PathBuf;
use uuid::Uuid;

pub struct ConfigBlock<'a> {
    pub image: &'a OciPackedImage,
    pub file: PathBuf,
    pub dir: PathBuf,
}

impl ConfigBlock<'_> {
    pub fn new<'a>(uuid: &Uuid, image: &'a OciPackedImage) -> Result<ConfigBlock<'a>> {
        let mut dir = std::env::temp_dir().clone();
        dir.push(format!("krata-cfg-{}", uuid));
        fs::create_dir_all(&dir)?;
        let mut file = dir.clone();
        file.push("config.squashfs");
        Ok(ConfigBlock { image, file, dir })
    }

    pub fn build(&self, launch_config: &LaunchInfo) -> Result<()> {
        trace!("build launch_config={:?}", launch_config);
        let config = self.image.config.raw();
        let launch = serde_json::to_string(launch_config)?;
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
            config,
            "/image/config.json",
            NodeHeader {
                permissions: 384,
                uid: 0,
                gid: 0,
                mtime: 0,
            },
        )?;
        writer.push_file(
            launch.as_bytes(),
            "/launch.json",
            NodeHeader {
                permissions: 384,
                uid: 0,
                gid: 0,
                mtime: 0,
            },
        )?;
        let mut file = File::create(&self.file)?;
        trace!("build write sqaushfs");
        writer.write(&mut file)?;
        trace!("build complete");
        Ok(())
    }
}
