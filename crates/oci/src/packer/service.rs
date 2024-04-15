use std::path::{Path, PathBuf};

use anyhow::{anyhow, Result};

use crate::{
    assemble::OciImageAssembler,
    fetch::OciImageFetcher,
    name::ImageName,
    progress::{OciBoundProgress, OciProgress, OciProgressContext},
    registry::OciPlatform,
};

use super::{cache::OciPackerCache, OciImagePacked, OciPackedFormat};

#[derive(Clone)]
pub struct OciPackerService {
    seed: Option<PathBuf>,
    platform: OciPlatform,
    cache: OciPackerCache,
    progress: OciProgressContext,
}

impl OciPackerService {
    pub fn new(
        seed: Option<PathBuf>,
        cache_dir: &Path,
        platform: OciPlatform,
        progress: OciProgressContext,
    ) -> Result<OciPackerService> {
        Ok(OciPackerService {
            seed,
            cache: OciPackerCache::new(cache_dir)?,
            platform,
            progress,
        })
    }

    pub async fn pack(
        &self,
        id: &str,
        name: ImageName,
        format: OciPackedFormat,
    ) -> Result<OciImagePacked> {
        let progress = OciProgress::new(id);
        let progress = OciBoundProgress::new(self.progress.clone(), progress);
        let fetcher =
            OciImageFetcher::new(self.seed.clone(), self.platform.clone(), progress.clone());
        let resolved = fetcher.resolve(name).await?;
        if let Some(cached) = self.cache.recall(&resolved, format).await? {
            return Ok(cached);
        }
        let assembler =
            OciImageAssembler::new(fetcher, resolved, progress.clone(), None, None).await?;
        let assembled = assembler.assemble().await?;
        let mut file = assembled
            .tmp_dir
            .clone()
            .ok_or(anyhow!("tmp_dir was missing when packing image"))?;
        file.push("image.pack");
        let target = file.clone();
        let directory = assembled.path.clone();
        tokio::task::spawn_blocking(move || {
            let packer = format.detect_best_backend().create();
            packer.pack(progress, &directory, &target)
        })
        .await??;
        let packed = OciImagePacked::new(
            assembled.digest.clone(),
            file,
            format,
            assembled.config.clone(),
            assembled.manifest.clone(),
        );
        let packed = self.cache.store(packed).await?;
        Ok(packed)
    }
}
