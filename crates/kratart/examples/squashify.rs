use std::{env::args, path::PathBuf};

use anyhow::Result;
use env_logger::Env;
use kratart::image::{cache::ImageCache, compiler::ImageCompiler, name::ImageName};
use tokio::fs;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let image = ImageName::parse(&args().nth(1).unwrap())?;
    let seed = args().nth(2).map(PathBuf::from);

    let cache_dir = PathBuf::from("krata-cache");
    if !cache_dir.exists() {
        fs::create_dir(&cache_dir).await?;
    }

    let cache = ImageCache::new(&cache_dir)?;
    let compiler = ImageCompiler::new(&cache, seed)?;
    let info = compiler.compile(&image).await?;
    println!(
        "generated squashfs of {} to {}",
        image,
        info.image_squashfs.to_string_lossy()
    );
    Ok(())
}
