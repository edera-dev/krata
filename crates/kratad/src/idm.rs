use anyhow::Result;
use kratart::channel::ChannelService;
use log::error;
use tokio::{sync::mpsc::Receiver, task::JoinHandle};

pub struct DaemonIdm {
    receiver: Receiver<(u32, Vec<u8>)>,
    task: JoinHandle<()>,
}

impl DaemonIdm {
    pub async fn new() -> Result<DaemonIdm> {
        let (service, receiver) = ChannelService::new("krata-channel".to_string()).await?;
        let task = service.launch().await?;
        Ok(DaemonIdm { receiver, task })
    }

    pub async fn launch(mut self) -> Result<JoinHandle<()>> {
        Ok(tokio::task::spawn(async move {
            if let Err(error) = self.process().await {
                error!("failed to process idm: {}", error);
            }
        }))
    }

    async fn process(&mut self) -> Result<()> {
        loop {
            let Some(_) = self.receiver.recv().await else {
                break;
            };
        }
        Ok(())
    }
}

impl Drop for DaemonIdm {
    fn drop(&mut self) {
        self.task.abort();
    }
}
