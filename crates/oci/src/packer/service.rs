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
}

impl OciPackerService {
    pub fn new(
        seed: Option<PathBuf>,
        cache_dir: &Path,
        platform: OciPlatform,
    ) -> Result<OciPackerService> {
        Ok(OciPackerService {
            seed,
            cache: OciPackerCache::new(cache_dir)?,
            platform,
        })
    }

    pub async fn recall(
        &self,
        digest: &str,
        format: OciPackedFormat,
    ) -> Result<Option<OciImagePacked>> {
        self.cache.recall(digest, format).await
    }

    pub async fn request(
        &self,
        name: ImageName,
        format: OciPackedFormat,
        progress_context: OciProgressContext,
    ) -> Result<OciImagePacked> {
        let progress = OciProgress::new();
        let progress = OciBoundProgress::new(progress_context.clone(), progress);
        let fetcher =
            OciImageFetcher::new(self.seed.clone(), self.platform.clone(), progress.clone());
        let resolved = fetcher.resolve(name).await?;
        if let Some(cached) = self.cache.recall(&resolved.digest, format).await? {
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
        let packer = format.backend().create();
        packer
            .pack(progress, assembled.vfs.clone(), &target)
            .await?;
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
