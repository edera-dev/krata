use std::{env::args, path::PathBuf};

use anyhow::Result;
use env_logger::Env;
use krataoci::{
    cache::ImageCache, compiler::ImageCompiler, name::ImageName, progress::OciProgressContext,
};
use tokio::{fs, sync::broadcast};

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

    let (sender, mut receiver) = broadcast::channel(1000);
    tokio::task::spawn(async move {
        loop {
            let Some(_) = receiver.recv().await.ok() else {
                break;
            };
        }
    });
    let context = OciProgressContext::new(sender);
    let compiler = ImageCompiler::new(&cache, seed, context)?;
    let info = compiler.compile(&image.to_string(), &image).await?;
    println!(
        "generated squashfs of {} to {}",
        image,
        info.image_squashfs.to_string_lossy()
    );
    Ok(())
}
