use std::{path::Path, sync::Arc};

use crate::idm::protocol::idm_packet::Content;

use super::protocol::{IdmEvent, IdmPacket};
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use log::{debug, error};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
use prost::Message;
use tokio::{
    fs::File,
    io::{unix::AsyncFd, AsyncReadExt, AsyncWriteExt},
    select,
    sync::{
        broadcast,
        mpsc::{channel, Receiver, Sender},
        Mutex,
    },
    task::JoinHandle,
};

const IDM_PACKET_QUEUE_LEN: usize = 100;

#[async_trait::async_trait]
pub trait IdmBackend: Send {
    async fn recv(&mut self) -> Result<IdmPacket>;
    async fn send(&mut self, packet: IdmPacket) -> Result<()>;
}

pub struct IdmFileBackend {
    fd: Arc<Mutex<AsyncFd<File>>>,
}

impl IdmFileBackend {
    pub async fn new(file: File) -> Result<IdmFileBackend> {
        IdmFileBackend::set_raw_port(&file)?;
        Ok(IdmFileBackend {
            fd: Arc::new(Mutex::new(AsyncFd::new(file)?)),
        })
    }

    fn set_raw_port(file: &File) -> Result<()> {
        let mut termios = tcgetattr(file)?;
        cfmakeraw(&mut termios);
        tcsetattr(file, SetArg::TCSANOW, &termios)?;
        Ok(())
    }
}

#[async_trait::async_trait]
impl IdmBackend for IdmFileBackend {
    async fn recv(&mut self) -> Result<IdmPacket> {
        let mut fd = self.fd.lock().await;
        let mut guard = fd.readable_mut().await?;
        let size = guard.get_inner_mut().read_u16_le().await?;
        if size == 0 {
            return Ok(IdmPacket::default());
        }
        let mut buffer = BytesMut::with_capacity(size as usize);
        guard.get_inner_mut().read_exact(&mut buffer).await?;
        match IdmPacket::decode(buffer) {
            Ok(packet) => Ok(packet),

            Err(error) => Err(anyhow!("received invalid idm packet: {}", error)),
        }
    }

    async fn send(&mut self, packet: IdmPacket) -> Result<()> {
        let mut fd = self.fd.lock().await;
        let data = packet.encode_to_vec();
        fd.get_mut().write_u16_le(data.len() as u16).await?;
        fd.get_mut().write_all(&data).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct IdmClient {
    event_receiver_sender: broadcast::Sender<IdmEvent>,
    tx_sender: Sender<IdmPacket>,
    task: Arc<JoinHandle<()>>,
}

impl Drop for IdmClient {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}

impl IdmClient {
    pub async fn new(backend: Box<dyn IdmBackend>) -> Result<IdmClient> {
        let (event_sender, event_receiver) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (tx_sender, tx_receiver) = channel(IDM_PACKET_QUEUE_LEN);
        let backend_event_sender = event_sender.clone();
        let task = tokio::task::spawn(async move {
            if let Err(error) =
                IdmClient::process(backend, backend_event_sender, event_receiver, tx_receiver).await
            {
                debug!("failed to handle idm client processing: {}", error);
            }
        });
        Ok(IdmClient {
            event_receiver_sender: event_sender.clone(),
            tx_sender,
            task: Arc::new(task),
        })
    }

    pub async fn open<P: AsRef<Path>>(path: P) -> Result<IdmClient> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(false)
            .open(path)
            .await?;
        let backend = IdmFileBackend::new(file).await?;
        IdmClient::new(Box::new(backend) as Box<dyn IdmBackend>).await
    }

    pub async fn emit(&self, event: IdmEvent) -> Result<()> {
        self.tx_sender
            .send(IdmPacket {
                content: Some(Content::Event(event)),
            })
            .await?;
        Ok(())
    }

    pub async fn subscribe(&self) -> Result<broadcast::Receiver<IdmEvent>> {
        Ok(self.event_receiver_sender.subscribe())
    }

    async fn process(
        mut backend: Box<dyn IdmBackend>,
        event_sender: broadcast::Sender<IdmEvent>,
        _event_receiver: broadcast::Receiver<IdmEvent>,
        mut receiver: Receiver<IdmPacket>,
    ) -> Result<()> {
        loop {
            select! {
                x = backend.recv() => match x {
                    Ok(packet) => {
                        match packet.content {
                            Some(Content::Event(event)) => {
                                let _ = event_sender.send(event);
                            },

                            _ => {},
                        }
                    },

                    Err(error) => {
                        return Err(anyhow!("failed to read idm client: {}", error));
                    }
                },
                x = receiver.recv() => match x {
                    Some(packet) => {
                        let length = packet.encoded_len();
                        if length > u16::MAX as usize {
                            error!("unable to send idm packet, packet size exceeded (tried to send {} bytes)", length);
                            continue;
                        }
                        backend.send(packet).await?;
                    },

                    None => {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
