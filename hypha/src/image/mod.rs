pub mod cache;
pub mod fetch;
pub mod name;

use crate::error::{HyphaError, Result};
use crate::image::cache::ImageCache;
use crate::image::fetch::RegistryClient;
use crate::image::name::ImageName;
use backhand::{FilesystemWriter, NodeHeader};
use flate2::read::GzDecoder;
use log::{debug, trace};
use oci_spec::image::{Descriptor, ImageConfiguration, ImageManifest, MediaType};
use std::fs;
use std::fs::File;
use std::io::{copy, BufReader, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use tar::Entry;
use uuid::Uuid;
use walkdir::WalkDir;

pub const IMAGE_SQUASHFS_VERSION: u64 = 1;

pub struct ImageInfo {
    pub image_squashfs: PathBuf,
    pub manifest: ImageManifest,
    pub config: ImageConfiguration,
}

impl ImageInfo {
    fn new(
        squashfs: PathBuf,
        manifest: ImageManifest,
        config: ImageConfiguration,
    ) -> Result<ImageInfo> {
        Ok(ImageInfo {
            image_squashfs: squashfs,
            manifest,
            config,
        })
    }
}

pub struct ImageCompiler<'a> {
    cache: &'a ImageCache,
}

impl ImageCompiler<'_> {
    pub fn new(cache: &ImageCache) -> Result<ImageCompiler> {
        Ok(ImageCompiler { cache })
    }

    pub fn compile(&self, image: &ImageName) -> Result<ImageInfo> {
        debug!("ImageCompiler compile image={image}");
        let mut tmp_dir = std::env::temp_dir().clone();
        tmp_dir.push(format!("hypha-compile-{}", Uuid::new_v4()));

        let mut image_dir = tmp_dir.clone();
        image_dir.push("image");
        fs::create_dir_all(&image_dir)?;

        let mut layer_dir = tmp_dir.clone();
        layer_dir.push("layer");
        fs::create_dir_all(&layer_dir)?;

        let mut squash_file = tmp_dir.clone();
        squash_file.push("image.squashfs");
        let info = self.download_and_compile(image, &layer_dir, &image_dir, &squash_file)?;
        fs::remove_dir_all(&tmp_dir)?;
        Ok(info)
    }

    fn download_and_compile(
        &self,
        image: &ImageName,
        layer_dir: &Path,
        image_dir: &PathBuf,
        squash_file: &PathBuf,
    ) -> Result<ImageInfo> {
        debug!(
            "ImageCompiler download manifest image={image}, image_dir={}",
            image_dir.to_str().unwrap()
        );
        let mut client = RegistryClient::new(image.registry_url()?)?;
        let manifest = client.get_manifest(&image.name, &image.reference)?;
        let manifest_serialized = serde_json::to_string(&manifest)?;
        let cache_key = format!(
            "manifest\n{}squashfs-version\n{}\n",
            manifest_serialized, IMAGE_SQUASHFS_VERSION
        );
        let cache_digest = sha256::digest(cache_key);

        if let Some(cached) = self.cache.recall(&cache_digest)? {
            return Ok(cached);
        }

        debug!(
            "ImageCompiler download config digest={} size={}",
            manifest.config().digest(),
            manifest.config().size(),
        );
        let config_bytes = client.get_blob(&image.name, manifest.config())?;
        let config: ImageConfiguration = serde_json::from_slice(&config_bytes)?;

        let mut layers: Vec<PathBuf> = Vec::new();
        for layer in manifest.layers() {
            let layer_path = self.download_layer(image, layer, layer_dir, &mut client)?;
            layers.push(layer_path);
        }

        for layer in layers {
            let mut file = File::open(&layer)?;
            self.process_whiteout_entries(&file, image_dir)?;
            file.seek(SeekFrom::Start(0))?;
            self.process_write_entries(&file, image_dir)?;
            drop(file);
            fs::remove_file(&layer)?;
        }

        self.squash(image_dir, squash_file)?;
        let info = ImageInfo::new(squash_file.clone(), manifest.clone(), config)?;
        self.cache.store(&cache_digest, &info)
    }

    fn process_whiteout_entries(&self, file: &File, image_dir: &PathBuf) -> Result<()> {
        let mut archive = tar::Archive::new(file);
        for entry in archive.entries()? {
            let entry = entry?;
            let dst = self.check_safe_entry(&entry, image_dir)?;
            let Some(name) = dst.file_name() else {
                return Err(HyphaError::new("unable to get file name"));
            };
            let Some(name) = name.to_str() else {
                return Err(HyphaError::new("unable to get file name as string"));
            };
            if !name.starts_with(".wh.") {
                continue;
            }
            let mut dst = dst.clone();
            dst.pop();

            let opaque = name == ".wh..wh..opq";

            if !opaque {
                dst.push(name);
                self.check_safe_path(&dst, image_dir)?;
            }

            if opaque {
                for entry in fs::read_dir(dst)? {
                    let entry = entry?;
                    let path = entry.path();
                    if path.is_file() {
                        fs::remove_file(&path)?;
                    } else {
                        fs::remove_dir_all(&path)?;
                    }
                }
            } else if dst.is_file() {
                fs::remove_file(&dst)?;
            } else {
                fs::remove_dir(&dst)?;
            }
        }
        Ok(())
    }

    fn process_write_entries(&self, file: &File, image_dir: &PathBuf) -> Result<()> {
        let mut archive = tar::Archive::new(file);
        for entry in archive.entries()? {
            let mut entry = entry?;
            let dst = self.check_safe_entry(&entry, image_dir)?;
            let Some(name) = dst.file_name() else {
                return Err(HyphaError::new("unable to get file name"));
            };
            let Some(name) = name.to_str() else {
                return Err(HyphaError::new("unable to get file name as string"));
            };
            if name.starts_with(".wh.") {
                continue;
            }
            entry.unpack(dst)?;
        }
        Ok(())
    }

    fn check_safe_entry(&self, entry: &Entry<&File>, image_dir: &PathBuf) -> Result<PathBuf> {
        let mut dst = image_dir.clone();
        dst.push(entry.path()?);
        self.check_safe_path(&dst, image_dir)?;
        Ok(dst)
    }

    fn check_safe_path(&self, dst: &PathBuf, image_dir: &PathBuf) -> Result<()> {
        let resolved = path_clean::clean(dst);
        if !resolved.starts_with(image_dir) {
            return Err(HyphaError::new("layer attempts to work outside image dir"));
        }
        Ok(())
    }

    fn download_layer(
        &self,
        image: &ImageName,
        layer: &Descriptor,
        layer_dir: &Path,
        client: &mut RegistryClient,
    ) -> Result<PathBuf> {
        debug!(
            "ImageCompiler download layer digest={} size={}",
            layer.digest(),
            layer.size()
        );
        let mut layer_path = layer_dir.to_path_buf();
        layer_path.push(layer.digest());
        let mut tmp_path = layer_dir.to_path_buf();
        tmp_path.push(format!("{}.tmp", layer.digest()));

        {
            let mut file = File::create(&layer_path)?;
            let size = client.write_blob(&image.name, layer, &mut file)?;
            if layer.size() as u64 != size {
                return Err(HyphaError::new(
                    "downloaded layer size differs from size in manifest",
                ));
            }
        }

        let compressed = match layer.media_type() {
            MediaType::ImageLayer => false,
            MediaType::ImageLayerGzip => {
                let reader = File::open(&layer_path)?;
                let mut decoder = GzDecoder::new(&reader);
                let mut writer = File::create(&tmp_path)?;
                copy(&mut decoder, &mut writer)?;
                writer.flush()?;
                true
            }
            MediaType::ImageLayerZstd => {
                let reader = File::open(&layer_path)?;
                let mut decoder = zstd::Decoder::new(&reader)?;
                let mut writer = File::create(&tmp_path)?;
                copy(&mut decoder, &mut writer)?;
                writer.flush()?;
                true
            }
            _ => return Err(HyphaError::new("found layer with unknown media type")),
        };

        if compressed {
            fs::rename(tmp_path, &layer_path)?;
        }
        Ok(layer_path)
    }

    fn squash(&self, image_dir: &PathBuf, squash_file: &PathBuf) -> Result<()> {
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
        trace!("ImageCompiler squash generate: {}", squash_file_path);
        writer.write(&mut out)?;
        Ok(())
    }
}
