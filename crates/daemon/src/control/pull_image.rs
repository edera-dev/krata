use crate::control::ApiError;
use crate::oci::convert_oci_progress;
use anyhow::Result;
use async_stream::try_stream;
use krata::v1::common::OciImageFormat;
use krata::v1::control::{PullImageReply, PullImageRequest};
use krataoci::name::ImageName;
use krataoci::packer::service::OciPackerService;
use krataoci::packer::{OciPackedFormat, OciPackedImage};
use krataoci::progress::{OciProgress, OciProgressContext};
use std::pin::Pin;
use tokio::select;
use tokio::task::JoinError;
use tokio_stream::Stream;
use tonic::Status;

enum PullImageSelect {
    Progress(Option<OciProgress>),
    Completed(Result<Result<OciPackedImage, anyhow::Error>, JoinError>),
}

pub struct PullImageRpc {
    packer: OciPackerService,
}

impl PullImageRpc {
    pub fn new(packer: OciPackerService) -> Self {
        Self { packer }
    }

    pub async fn process(
        self,
        request: PullImageRequest,
    ) -> Result<Pin<Box<dyn Stream<Item = Result<PullImageReply, Status>> + Send + 'static>>> {
        let name = ImageName::parse(&request.image)?;
        let format = match request.format() {
            OciImageFormat::Unknown => OciPackedFormat::Squashfs,
            OciImageFormat::Squashfs => OciPackedFormat::Squashfs,
            OciImageFormat::Erofs => OciPackedFormat::Erofs,
            OciImageFormat::Tar => OciPackedFormat::Tar,
        };
        let (context, mut receiver) = OciProgressContext::create();
        let our_packer = self.packer;

        let output = try_stream! {
            let mut task = tokio::task::spawn(async move {
                our_packer.request(name, format, request.overwrite_cache, request.update, context).await
            });
            let abort_handle = task.abort_handle();
            let _task_cancel_guard = scopeguard::guard(abort_handle, |handle| {
                handle.abort();
            });

            loop {
                let what = select! {
                    x = receiver.changed() => match x {
                        Ok(_) => PullImageSelect::Progress(Some(receiver.borrow_and_update().clone())),
                        Err(_) => PullImageSelect::Progress(None),
                    },
                    x = &mut task => PullImageSelect::Completed(x),
                };
                match what {
                    PullImageSelect::Progress(Some(progress)) => {
                        let reply = PullImageReply {
                            progress: Some(convert_oci_progress(progress)),
                            digest: String::new(),
                            format: OciImageFormat::Unknown.into(),
                        };
                        yield reply;
                    },

                    PullImageSelect::Completed(result) => {
                        let result = result.map_err(|err| ApiError {
                            message: err.to_string(),
                        })?;
                        let packed = result.map_err(|err| ApiError {
                            message: err.to_string(),
                        })?;
                        let reply = PullImageReply {
                            progress: None,
                            digest: packed.digest,
                            format: match packed.format {
                                OciPackedFormat::Squashfs => OciImageFormat::Squashfs.into(),
                                OciPackedFormat::Erofs => OciImageFormat::Erofs.into(),
                                OciPackedFormat::Tar => OciImageFormat::Tar.into(),
                            },
                        };
                        yield reply;
                        break;
                    },

                    _ => {
                        continue;
                    }
                }
            }
        };
        Ok(Box::pin(output))
    }
}
