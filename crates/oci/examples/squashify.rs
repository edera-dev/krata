use std::{env::args, path::PathBuf};

use anyhow::Result;
use env_logger::Env;
use krataoci::{
    name::ImageName,
    packer::{service::OciPackerService, OciPackedFormat},
    progress::OciProgressContext,
    registry::OciPlatform,
};
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

    let (context, mut receiver) = OciProgressContext::create();
    tokio::task::spawn(async move {
        loop {
            let Ok(mut progress) = receiver.recv().await else {
                return;
            };

            let mut drain = 0;
            loop {
                if drain >= 10 {
                    break;
                }

                if let Ok(latest) = receiver.try_recv() {
                    progress = latest;
                } else {
                    break;
                }

                drain += 1;
            }

            println!("phase {:?}", progress.phase);
            for (id, layer) in &progress.layers {
                println!(
                    "{} {:?} {} of {}",
                    id, layer.phase, layer.value, layer.total
                )
            }
        }
    });
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
