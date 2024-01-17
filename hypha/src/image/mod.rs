use crate::error::{HyphaError, Result};
use backhand::{FilesystemWriter, NodeHeader};
use log::{debug, trace};
use oci_spec::image::MediaType;
use ocipkg::distribution::Client;
use ocipkg::error::Error;
use ocipkg::{Digest, ImageName};
use std::fs;
use std::fs::File;
use std::io::BufReader;
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::PathBuf;
use uuid::Uuid;
use walkdir::WalkDir;

pub struct ImageCompiler {}

impl ImageCompiler {
    pub fn new() -> Result<ImageCompiler> {
        Ok(ImageCompiler {})
    }

    pub fn compile(&self, image: &ImageName) -> Result<String> {
        debug!("ImageCompiler compile image={image}");
        let mut tmp_dir = std::env::temp_dir().clone();
        tmp_dir.push(format!("hypha-compile-{}", Uuid::new_v4()));
        let mut image_dir = tmp_dir.clone();
        image_dir.push("image");
        fs::create_dir_all(&image_dir)?;
        let mut squash_file = tmp_dir.clone();
        squash_file.push("image.squashfs");
        self.download(image, &image_dir)?;
        self.squash(&image_dir, &squash_file)
    }

    fn download(&self, image: &ImageName, image_dir: &PathBuf) -> Result<()> {
        debug!(
            "ImageCompiler download image={image}, image_dir={}",
            image_dir.to_str().unwrap()
        );
        let ImageName {
            name, reference, ..
        } = image;
        let mut client = Client::new(image.registry_url()?, name.clone())?;
        let manifest = client.get_manifest(reference)?;
        for layer in manifest.layers() {
            debug!(
                "ImageCompiler download start digest={} size={}",
                layer.digest(),
                layer.size()
            );
            let blob = client.get_blob(&Digest::new(layer.digest())?)?;
            match layer.media_type() {
                MediaType::ImageLayerGzip => {}
                MediaType::Other(ty) => {
                    if !ty.ends_with("tar.gzip") {
                        continue;
                    }
                }
                _ => continue,
            }
            debug!(
                "ImageCompiler download unpack digest={} size={}",
                layer.digest(),
                layer.size()
            );
            let buf = flate2::read::GzDecoder::new(blob.as_slice());
            tar::Archive::new(buf).unpack(image_dir)?;
            debug!(
                "ImageCompiler download end digest={} size={}",
                layer.digest(),
                layer.size()
            );
            return Ok(());
        }
        Err(Error::MissingLayer.into())
    }

    fn squash(&self, image_dir: &PathBuf, squash_file: &PathBuf) -> Result<String> {
        let mut writer = FilesystemWriter::default();
        let walk = WalkDir::new(image_dir).follow_links(false);
        for entry in walk {
            let entry = entry?;
            let rel = entry
                .path()
                .strip_prefix(image_dir)?
                .to_str()
                .ok_or_else(|| HyphaError::new("failed to strip prefix of tmpdir"))?;
            let rel = format!("/{}", rel);
            trace!("ImageCompiler squash write {}", rel);
            let typ = entry.file_type();
            let metadata = fs::symlink_metadata(entry.path())?;
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
                let symlink = fs::read_link(entry.path())?;
                let symlink = symlink
                    .to_str()
                    .ok_or_else(|| HyphaError::new("failed to read symlink"))?;
                writer.push_symlink(symlink, rel, header)?;
            } else if typ.is_dir() {
                writer.push_dir(rel, header)?;
            } else if typ.is_file() {
                let reader = BufReader::new(File::open(entry.path())?);
                writer.push_file(reader, rel, header)?;
            } else if typ.is_block_device() {
                let device = metadata.dev();
                writer.push_block_device(device as u32, rel, header)?;
            } else if typ.is_char_device() {
                let device = metadata.dev();
                writer.push_char_device(device as u32, rel, header)?;
            } else {
                return Err(HyphaError::new("invalid file type"));
            }
        }

        fs::remove_dir_all(image_dir)?;

        let squash_file_path = squash_file
            .to_str()
            .ok_or_else(|| HyphaError::new("failed to convert squashfs string"))?;

        let mut out = File::create(squash_file)?;
        trace!("ImageCompiler squash generateI : {}", squash_file_path);
        writer.write(&mut out)?;

        Ok(squash_file_path.to_string())
    }
}
