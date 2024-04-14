use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};

use super::protocol::{
    idm_packet::Content, idm_request::Request, idm_response::Response, IdmEvent, IdmPacket,
    IdmRequest, IdmResponse,
};
use anyhow::{anyhow, Result};
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
        oneshot, Mutex,
    },
    task::JoinHandle,
    time::timeout,
};

type RequestMap = Arc<Mutex<HashMap<u64, oneshot::Sender<IdmResponse>>>>;

const IDM_PACKET_QUEUE_LEN: usize = 100;
const IDM_REQUEST_TIMEOUT_SECS: u64 = 10;
const IDM_PACKET_MAX_SIZE: usize = 20 * 1024 * 1024;

#[async_trait::async_trait]
pub trait IdmBackend: Send {
    async fn recv(&mut self) -> Result<IdmPacket>;
    async fn send(&mut self, packet: IdmPacket) -> Result<()>;
}

pub struct IdmFileBackend {
    read_fd: Arc<Mutex<AsyncFd<File>>>,
    write: Arc<Mutex<File>>,
}

impl IdmFileBackend {
    pub async fn new(read_file: File, write_file: File) -> Result<IdmFileBackend> {
        IdmFileBackend::set_raw_port(&read_file)?;
        IdmFileBackend::set_raw_port(&write_file)?;
        Ok(IdmFileBackend {
            read_fd: Arc::new(Mutex::new(AsyncFd::new(read_file)?)),
            write: Arc::new(Mutex::new(write_file)),
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
        let mut fd = self.read_fd.lock().await;
        let mut guard = fd.readable_mut().await?;
        let size = guard.get_inner_mut().read_u32_le().await?;
        if size == 0 {
            return Ok(IdmPacket::default());
        }
        let mut buffer = vec![0u8; size as usize];
        guard.get_inner_mut().read_exact(&mut buffer).await?;
        match IdmPacket::decode(buffer.as_slice()) {
            Ok(packet) => Ok(packet),
            Err(error) => Err(anyhow!("received invalid idm packet: {}", error)),
        }
    }

    async fn send(&mut self, packet: IdmPacket) -> Result<()> {
        let mut file = self.write.lock().await;
        let data = packet.encode_to_vec();
        file.write_u32_le(data.len() as u32).await?;
        file.write_all(&data).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct IdmClient {
    request_backend_sender: broadcast::Sender<IdmRequest>,
    next_request_id: Arc<Mutex<u64>>,
    event_receiver_sender: broadcast::Sender<IdmEvent>,
    tx_sender: Sender<IdmPacket>,
    requests: RequestMap,
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
        let requests = Arc::new(Mutex::new(HashMap::new()));
        let (event_sender, event_receiver) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (internal_request_backend_sender, _) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (tx_sender, tx_receiver) = channel(IDM_PACKET_QUEUE_LEN);
        let backend_event_sender = event_sender.clone();
        let request_backend_sender = internal_request_backend_sender.clone();
        let requests_for_client = requests.clone();
        let task = tokio::task::spawn(async move {
            if let Err(error) = IdmClient::process(
                backend,
                backend_event_sender,
                requests,
                internal_request_backend_sender,
                event_receiver,
                tx_receiver,
            )
            .await
            {
                debug!("failed to handle idm client processing: {}", error);
            }
        });
        Ok(IdmClient {
            next_request_id: Arc::new(Mutex::new(0)),
            event_receiver_sender: event_sender.clone(),
            request_backend_sender,
            requests: requests_for_client,
            tx_sender,
            task: Arc::new(task),
        })
    }

    pub async fn open<P: AsRef<Path>>(path: P) -> Result<IdmClient> {
        let read_file = File::options()
            .read(true)
            .write(false)
            .create(false)
            .open(&path)
            .await?;
        let write_file = File::options()
            .read(false)
            .write(true)
            .create(false)
            .open(path)
            .await?;
        let backend = IdmFileBackend::new(read_file, write_file).await?;
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

    pub async fn requests(&self) -> Result<broadcast::Receiver<IdmRequest>> {
        Ok(self.request_backend_sender.subscribe())
    }

    pub async fn respond(&self, id: u64, response: Response) -> Result<()> {
        let packet = IdmPacket {
            content: Some(Content::Response(IdmResponse {
                id,
                response: Some(response),
            })),
        };
        self.tx_sender.send(packet).await?;
        Ok(())
    }

    pub async fn subscribe(&self) -> Result<broadcast::Receiver<IdmEvent>> {
        Ok(self.event_receiver_sender.subscribe())
    }

    pub async fn send(&self, request: Request) -> Result<Response> {
        let (sender, receiver) = oneshot::channel::<IdmResponse>();
        let req = {
            let mut guard = self.next_request_id.lock().await;
            let req = *guard;
            *guard = req.wrapping_add(1);
            req
        };
        let mut requests = self.requests.lock().await;
        requests.insert(req, sender);
        drop(requests);
        let success = AtomicBool::new(false);
        let _guard = scopeguard::guard(self.requests.clone(), |requests| {
            if success.load(Ordering::Acquire) {
                return;
            }
            tokio::task::spawn(async move {
                let mut requests = requests.lock().await;
                requests.remove(&req);
            });
        });
        self.tx_sender
            .send(IdmPacket {
                content: Some(Content::Request(IdmRequest {
                    id: req,
                    request: Some(request),
                })),
            })
            .await?;

        let response = timeout(Duration::from_secs(IDM_REQUEST_TIMEOUT_SECS), receiver).await??;
        success.store(true, Ordering::Release);
        if let Some(response) = response.response {
            Ok(response)
        } else {
            Err(anyhow!("response did not contain any content"))
        }
    }

    async fn process(
        mut backend: Box<dyn IdmBackend>,
        event_sender: broadcast::Sender<IdmEvent>,
        requests: RequestMap,
        request_backend_sender: broadcast::Sender<IdmRequest>,
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

                            Some(Content::Request(request)) => {
                                let _ = request_backend_sender.send(request);
                            },

                            Some(Content::Response(response)) => {
                                let mut requests = requests.lock().await;
                                if let Some(sender) = requests.remove(&response.id) {
                                    drop(requests);
                                    let _ = sender.send(response);
                                }
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
                        if length > IDM_PACKET_MAX_SIZE {
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
