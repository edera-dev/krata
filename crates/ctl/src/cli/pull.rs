use anyhow::Result;
use clap::{Parser, ValueEnum};
use krata::v1::{
    common::OciImageFormat,
    control::{control_service_client::ControlServiceClient, PullImageRequest},
};

use tonic::transport::Channel;

use crate::pull::pull_interactive_progress;

#[derive(ValueEnum, Clone, Debug, PartialEq, Eq)]
pub enum PullImageFormat {
    Squashfs,
    Erofs,
    Tar,
}

#[derive(Parser)]
#[command(about = "Pull an image into the cache")]
pub struct PullCommand {
    #[arg(help = "Image name")]
    image: String,
    #[arg(short = 's', long, default_value = "squashfs", help = "Image format")]
    image_format: PullImageFormat,
}

impl PullCommand {
    pub async fn run(self, mut client: ControlServiceClient<Channel>) -> Result<()> {
        let response = client
            .pull_image(PullImageRequest {
                image: self.image.clone(),
                format: match self.image_format {
                    PullImageFormat::Squashfs => OciImageFormat::Squashfs.into(),
                    PullImageFormat::Erofs => OciImageFormat::Erofs.into(),
                    PullImageFormat::Tar => OciImageFormat::Tar.into(),
                },
            })
            .await?;
        let reply = pull_interactive_progress(response.into_inner()).await?;
        println!("{}", reply.digest);
        Ok(())
    }
}
