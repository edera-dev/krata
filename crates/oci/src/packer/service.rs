use std::{
    collections::{hash_map::Entry, HashMap},
    fmt::Display,
    path::{Path, PathBuf},
    sync::Arc,
};

use anyhow::{anyhow, Result};
use tokio::{
    sync::{watch, Mutex},
    task::JoinHandle,
};

use crate::{
    assemble::OciImageAssembler,
    fetch::{OciImageFetcher, OciResolvedImage},
    name::ImageName,
    progress::{OciBoundProgress, OciProgress, OciProgressContext},
    registry::OciPlatform,
};

use log::{error, info, warn};

use super::{cache::OciPackerCache, OciPackedFormat, OciPackedImage};

pub struct OciPackerTask {
    progress: OciBoundProgress,
    watch: watch::Sender<Option<Result<OciPackedImage>>>,
    task: JoinHandle<()>,
}

#[derive(Clone)]
pub struct OciPackerService {
    seed: Option<PathBuf>,
    platform: OciPlatform,
    cache: OciPackerCache,
    tasks: Arc<Mutex<HashMap<OciPackerTaskKey, OciPackerTask>>>,
}

impl OciPackerService {
    pub async fn new(
        seed: Option<PathBuf>,
        cache_dir: &Path,
        platform: OciPlatform,
    ) -> Result<OciPackerService> {
        Ok(OciPackerService {
            seed,
            cache: OciPackerCache::new(cache_dir).await?,
            platform,
            tasks: Arc::new(Mutex::new(HashMap::new())),
        })
    }

    pub async fn recall(
        &self,
        digest: &str,
        format: OciPackedFormat,
    ) -> Result<Option<OciPackedImage>> {
        self.cache
            .recall(ImageName::parse("cached:latest")?, digest, format)
            .await
    }

    pub async fn request(
        &self,
        name: ImageName,
        format: OciPackedFormat,
        overwrite: bool,
        progress_context: OciProgressContext,
    ) -> Result<OciPackedImage> {
        let progress = OciProgress::new();
        let progress = OciBoundProgress::new(progress_context.clone(), progress);
        let fetcher =
            OciImageFetcher::new(self.seed.clone(), self.platform.clone(), progress.clone());
        let resolved = fetcher.resolve(name.clone()).await?;
        let key = OciPackerTaskKey {
            digest: resolved.digest.clone(),
            format,
        };
        let (progress_copy_task, mut receiver) = match self.tasks.lock().await.entry(key.clone()) {
            Entry::Occupied(entry) => {
                let entry = entry.get();
                (
                    Some(entry.progress.also_update(progress_context).await),
                    entry.watch.subscribe(),
                )
            }

            Entry::Vacant(entry) => {
                let task = self
                    .clone()
                    .launch(
                        name,
                        key.clone(),
                        format,
                        overwrite,
                        resolved,
                        fetcher,
                        progress.clone(),
                    )
                    .await;
                let (watch, receiver) = watch::channel(None);

                let task = OciPackerTask {
                    progress: progress.clone(),
                    task,
                    watch,
                };
                entry.insert(task);
                (None, receiver)
            }
        };

        let _progress_task_guard = scopeguard::guard(progress_copy_task, |task| {
            if let Some(task) = task {
                task.abort();
            }
        });

        let _task_cancel_guard = scopeguard::guard(self.clone(), |service| {
            service.maybe_cancel_task(key);
        });

        loop {
            receiver.changed().await?;
            let current = receiver.borrow_and_update();
            if current.is_some() {
                return current
                    .as_ref()
                    .map(|x| x.as_ref().map_err(|err| anyhow!("{}", err)).cloned())
                    .unwrap();
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    async fn launch(
        self,
        name: ImageName,
        key: OciPackerTaskKey,
        format: OciPackedFormat,
        overwrite: bool,
        resolved: OciResolvedImage,
        fetcher: OciImageFetcher,
        progress: OciBoundProgress,
    ) -> JoinHandle<()> {
        info!("packer task {} started", key);
        tokio::task::spawn(async move {
            let _task_drop_guard =
                scopeguard::guard((key.clone(), self.clone()), |(key, service)| {
                    service.ensure_task_gone(key);
                });
            if let Err(error) = self
                .task(
                    name,
                    key.clone(),
                    format,
                    overwrite,
                    resolved,
                    fetcher,
                    progress,
                )
                .await
            {
                self.finish(&key, Err(error)).await;
            }
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn task(
        &self,
        name: ImageName,
        key: OciPackerTaskKey,
        format: OciPackedFormat,
        overwrite: bool,
        resolved: OciResolvedImage,
        fetcher: OciImageFetcher,
        progress: OciBoundProgress,
    ) -> Result<()> {
        if !overwrite {
            if let Some(cached) = self
                .cache
                .recall(name.clone(), &resolved.digest, format)
                .await?
            {
                self.finish(&key, Ok(cached)).await;
                return Ok(());
            }
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
        let packed = OciPackedImage::new(
            name,
            assembled.digest.clone(),
            file,
            format,
            assembled.descriptor.clone(),
            assembled.config.clone(),
            assembled.manifest.clone(),
        );
        let packed = self.cache.store(packed).await?;
        self.finish(&key, Ok(packed)).await;
        Ok(())
    }

    async fn finish(&self, key: &OciPackerTaskKey, result: Result<OciPackedImage>) {
        let Some(task) = self.tasks.lock().await.remove(key) else {
            error!("packer task {} was not found when task completed", key);
            return;
        };

        match result.as_ref() {
            Ok(_) => {
                info!("packer task {} completed", key);
            }

            Err(err) => {
                warn!("packer task {} failed: {}", key, err);
            }
        }

        task.watch.send_replace(Some(result));
    }

    fn maybe_cancel_task(self, key: OciPackerTaskKey) {
        tokio::task::spawn(async move {
            let tasks = self.tasks.lock().await;
            if let Some(task) = tasks.get(&key) {
                if task.watch.is_closed() {
                    task.task.abort();
                }
            }
        });
    }

    fn ensure_task_gone(self, key: OciPackerTaskKey) {
        tokio::task::spawn(async move {
            let mut tasks = self.tasks.lock().await;
            if let Some(task) = tasks.remove(&key) {
                warn!("packer task {} aborted", key);
                task.watch.send_replace(Some(Err(anyhow!("task aborted"))));
            }
        });
    }
}

#[derive(Debug, Clone, Eq, PartialEq, Hash)]
struct OciPackerTaskKey {
    digest: String,
    format: OciPackedFormat,
}

impl Display for OciPackerTaskKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}:{}", self.digest, self.format.extension()))
    }
}
