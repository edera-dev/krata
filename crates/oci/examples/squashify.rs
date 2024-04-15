use std::{env::args, path::PathBuf};

use anyhow::Result;
use env_logger::Env;
use krataoci::{
    name::ImageName,
    packer::{service::OciPackerService, OciPackedFormat},
    progress::{OciProgress, OciProgressContext},
    registry::OciPlatform,
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

    let (sender, mut receiver) = broadcast::channel::<OciProgress>(1000);
    tokio::task::spawn(async move {
        loop {
            let Some(progress) = receiver.recv().await.ok() else {
                break;
            };
            println!("phase {:?}", progress.phase);
            for (id, layer) in progress.layers {
                println!(
                    "{} {:?} {} of {}",
                    id, layer.phase, layer.value, layer.total
                )
            }
        }
    });
    let context = OciProgressContext::new(sender);
    let service = OciPackerService::new(seed, &cache_dir, OciPlatform::current())?;
    let packed = service
        .request(image.clone(), OciPackedFormat::Squashfs, context)
        .await?;
    println!(
        "generated squashfs of {} to {}",
        image,
        packed.path.to_string_lossy()
    );
    Ok(())
}
