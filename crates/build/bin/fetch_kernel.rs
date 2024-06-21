use std::{
    env::{self, args},
    path::PathBuf,
};

use anyhow::{anyhow, Result};
use env_logger::Env;
use krataoci::{
    name::ImageName,
    packer::{service::OciPackerService, OciPackedFormat},
    progress::OciProgressContext,
    registry::OciPlatform,
};
use oci_spec::image::{Arch, Os};
use tokio::{
    fs::{self, File},
    io::BufReader,
};
use tokio_stream::StreamExt;
use tokio_tar::Archive;
use uuid::Uuid;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("warn")).init();
    fs::create_dir_all("target/kernel").await?;

    let arch = env::var("TARGET_ARCH").map_err(|_| anyhow!("missing TARGET_ARCH env var"))?;
    println!("kernel architecture: {}", arch);
    let platform = OciPlatform::new(
        Os::Linux,
        match arch.as_str() {
            "x86_64" => Arch::Amd64,
            "aarch64" => Arch::ARM64,
            _ => {
                return Err(anyhow!("unknown architecture '{}'", arch));
            }
        },
    );

    let image = ImageName::parse(&args().nth(1).unwrap())?;
    let mut cache_dir = std::env::temp_dir().clone();
    cache_dir.push(format!("krata-cache-{}", Uuid::new_v4()));
    fs::create_dir_all(&cache_dir).await?;

    let _delete_cache_dir = scopeguard::guard(cache_dir.clone(), |dir| {
        let _ = std::fs::remove_dir_all(dir);
    });

    let (context, _) = OciProgressContext::create();
    let service = OciPackerService::new(None, &cache_dir, platform).await?;
    let packed = service
        .request(image.clone(), OciPackedFormat::Tar, false, context)
        .await?;
    let annotations = packed
        .manifest
        .item()
        .annotations()
        .clone()
        .unwrap_or_default();
    let Some(format) = annotations.get("dev.edera.kernel.format") else {
        return Err(anyhow!(
            "image manifest missing 'dev.edera.kernel.format' annotation"
        ));
    };
    let Some(version) = annotations.get("dev.edera.kernel.version") else {
        return Err(anyhow!(
            "image manifest missing 'dev.edera.kernel.version' annotation"
        ));
    };
    let Some(flavor) = annotations.get("dev.edera.kernel.flavor") else {
        return Err(anyhow!(
            "image manifest missing 'dev.edera.kernel.flavor' annotation"
        ));
    };

    if format != "1" {
        return Err(anyhow!("kernel format version '{}' is unknown", format));
    }

    let file = BufReader::new(File::open(packed.path).await?);
    let mut archive = Archive::new(file);
    let mut entries = archive.entries()?;

    let kernel_image_tar_path = PathBuf::from("kernel/image");
    let kernel_addons_tar_path = PathBuf::from("kernel/addons.squashfs");
    let kernel_image_out_path = PathBuf::from(format!("target/kernel/kernel-{}", arch));
    let kernel_addons_out_path = PathBuf::from(format!("target/kernel/addons-{}.squashfs", arch));

    if kernel_image_out_path.exists() {
        fs::remove_file(&kernel_image_out_path).await?;
    }

    if kernel_addons_out_path.exists() {
        fs::remove_file(&kernel_addons_out_path).await?;
    }

    while let Some(entry) = entries.next().await {
        let mut entry = entry?;
        let path = entry.path()?.to_path_buf();

        if !entry.header().entry_type().is_file() {
            continue;
        }

        if path == kernel_image_tar_path {
            entry.unpack(&kernel_image_out_path).await?;
        } else if path == kernel_addons_tar_path {
            entry.unpack(&kernel_addons_out_path).await?;
        }
    }

    if !kernel_image_out_path.exists() {
        return Err(anyhow!("image did not contain a file named /kernel/image"));
    }

    println!("kernel version: v{}", version);
    println!("kernel flavor: {}", flavor);

    Ok(())
}
