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
            if (receiver.changed().await).is_err() {
                break;
            }
            let progress = receiver.borrow_and_update();
            println!("phase {:?}", progress.phase);
            for (id, layer) in &progress.layers {
                println!("{} {:?} {:?}", id, layer.phase, layer.indication,)
            }
        }
    });
    let service = OciPackerService::new(seed, &cache_dir, OciPlatform::current()).await?;
    let packed = service
        .request(
            image.clone(),
            OciPackedFormat::Squashfs,
            false,
            true,
            context,
        )
        .await?;
    println!(
        "generated squashfs of {} to {}",
        image,
        packed.path.to_string_lossy()
    );
    Ok(())
}
