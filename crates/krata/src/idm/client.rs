use std::path::Path;

use super::protocol::IdmPacket;
use anyhow::{anyhow, Result};
use bytes::BytesMut;
use log::{debug, error};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
use prost::Message;
use tokio::{
    fs::File,
    io::{unix::AsyncFd, AsyncReadExt, AsyncWriteExt},
    select,
    sync::mpsc::{channel, Receiver, Sender},
    task::JoinHandle,
};

const IDM_PACKET_QUEUE_LEN: usize = 100;

pub struct IdmClient {
    pub receiver: Receiver<IdmPacket>,
    pub sender: Sender<IdmPacket>,
    task: JoinHandle<()>,
}

impl Drop for IdmClient {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl IdmClient {
    pub async fn open<P: AsRef<Path>>(path: P) -> Result<IdmClient> {
        let file = File::options()
            .read(true)
            .write(true)
            .create(false)
            .open(path)
            .await?;
        IdmClient::set_raw_port(&file)?;
        let (rx_sender, rx_receiver) = channel(IDM_PACKET_QUEUE_LEN);
        let (tx_sender, tx_receiver) = channel(IDM_PACKET_QUEUE_LEN);
        let task = tokio::task::spawn(async move {
            if let Err(error) = IdmClient::process(file, rx_sender, tx_receiver).await {
                debug!("failed to handle idm client processing: {}", error);
            }
        });
        Ok(IdmClient {
            receiver: rx_receiver,
            sender: tx_sender,
            task,
        })
    }

    fn set_raw_port(file: &File) -> Result<()> {
        let mut termios = tcgetattr(file)?;
        cfmakeraw(&mut termios);
        tcsetattr(file, SetArg::TCSANOW, &termios)?;
        Ok(())
    }

    async fn process(
        file: File,
        sender: Sender<IdmPacket>,
        mut receiver: Receiver<IdmPacket>,
    ) -> Result<()> {
        let mut file = AsyncFd::new(file)?;
        loop {
            select! {
                x = file.readable_mut() => match x {
                    Ok(mut guard) => {
                        let size = guard.get_inner_mut().read_u16_le().await?;
                        if size == 0 {
                            continue;
                        }
                        let mut buffer = BytesMut::with_capacity(size as usize);
                        guard.get_inner_mut().read_exact(&mut buffer).await?;
                        match IdmPacket::decode(buffer) {
                            Ok(packet) => {
                                sender.send(packet).await?;
                            },

                            Err(error) => {
                                error!("received invalid idm packet: {}", error);
                            }
                        }
                    },

                    Err(error) => {
                        return Err(anyhow!("failed to read idm client: {}", error));
                    }
                },
                x = receiver.recv() => match x {
                    Some(packet) => {
                        let data = packet.encode_to_vec();
                        if data.len() > u16::MAX as usize {
                            error!("unable to send idm packet, packet size exceeded (tried to send {} bytes)", data.len());
                            continue;
                        }
                        file.get_mut().write_u16_le(data.len() as u16).await?;
                        file.get_mut().write_all(&data).await?;
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
