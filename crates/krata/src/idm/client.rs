use std::{
    collections::HashMap,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
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
        mpsc::{self, Receiver, Sender},
        oneshot, Mutex,
    },
    task::JoinHandle,
    time::timeout,
};

use super::{
    internal,
    serialize::{IdmRequest, IdmSerializable},
    transport::{IdmTransportPacket, IdmTransportPacketForm},
};

type RequestMap<R> = Arc<Mutex<HashMap<u64, oneshot::Sender<<R as IdmRequest>::Response>>>>;
pub type IdmInternalClient = IdmClient<internal::Request, internal::Event>;

const IDM_PACKET_QUEUE_LEN: usize = 100;
const IDM_REQUEST_TIMEOUT_SECS: u64 = 10;
const IDM_PACKET_MAX_SIZE: usize = 20 * 1024 * 1024;

#[async_trait::async_trait]
pub trait IdmBackend: Send {
    async fn recv(&mut self) -> Result<IdmTransportPacket>;
    async fn send(&mut self, packet: IdmTransportPacket) -> Result<()>;
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
    async fn recv(&mut self) -> Result<IdmTransportPacket> {
        let mut fd = self.read_fd.lock().await;
        let mut guard = fd.readable_mut().await?;
        let b1 = guard.get_inner_mut().read_u8().await?;
        if b1 != 0xff {
            return Ok(IdmTransportPacket::default());
        }
        let b2 = guard.get_inner_mut().read_u8().await?;
        if b2 != 0xff {
            return Ok(IdmTransportPacket::default());
        }
        let size = guard.get_inner_mut().read_u32_le().await?;
        if size == 0 {
            return Ok(IdmTransportPacket::default());
        }
        let mut buffer = vec![0u8; size as usize];
        guard.get_inner_mut().read_exact(&mut buffer).await?;
        match IdmTransportPacket::decode(buffer.as_slice()) {
            Ok(packet) => Ok(packet),
            Err(error) => Err(anyhow!("received invalid idm packet: {}", error)),
        }
    }

    async fn send(&mut self, packet: IdmTransportPacket) -> Result<()> {
        let mut file = self.write.lock().await;
        let data = packet.encode_to_vec();
        file.write_all(&[0xff, 0xff]).await?;
        file.write_u32_le(data.len() as u32).await?;
        file.write_all(&data).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct IdmClient<R: IdmRequest, E: IdmSerializable> {
    channel: u64,
    request_backend_sender: broadcast::Sender<(u64, R)>,
    next_request_id: Arc<Mutex<u64>>,
    event_receiver_sender: broadcast::Sender<E>,
    tx_sender: Sender<IdmTransportPacket>,
    requests: RequestMap<R>,
    task: Arc<JoinHandle<()>>,
}

impl<R: IdmRequest, E: IdmSerializable> Drop for IdmClient<R, E> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}

impl<R: IdmRequest, E: IdmSerializable> IdmClient<R, E> {
    pub async fn new(channel: u64, backend: Box<dyn IdmBackend>) -> Result<Self> {
        let requests = Arc::new(Mutex::new(HashMap::new()));
        let (event_sender, event_receiver) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (internal_request_backend_sender, _) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (tx_sender, tx_receiver) = mpsc::channel(IDM_PACKET_QUEUE_LEN);
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
            channel,
            next_request_id: Arc::new(Mutex::new(0)),
            event_receiver_sender: event_sender.clone(),
            request_backend_sender,
            requests: requests_for_client,
            tx_sender,
            task: Arc::new(task),
        })
    }

    pub async fn open<P: AsRef<Path>>(channel: u64, path: P) -> Result<Self> {
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
        IdmClient::new(channel, Box::new(backend) as Box<dyn IdmBackend>).await
    }

    pub async fn emit<T: IdmSerializable>(&self, event: T) -> Result<()> {
        let id = {
            let mut guard = self.next_request_id.lock().await;
            let req = *guard;
            *guard = req.wrapping_add(1);
            req
        };
        self.tx_sender
            .send(IdmTransportPacket {
                id,
                form: IdmTransportPacketForm::Event.into(),
                channel: self.channel,
                data: event.encode()?,
            })
            .await?;
        Ok(())
    }

    pub async fn requests(&self) -> Result<broadcast::Receiver<(u64, R)>> {
        Ok(self.request_backend_sender.subscribe())
    }

    pub async fn respond<T: IdmSerializable>(&self, id: u64, response: T) -> Result<()> {
        let packet = IdmTransportPacket {
            id,
            form: IdmTransportPacketForm::Response.into(),
            channel: self.channel,
            data: response.encode()?,
        };
        self.tx_sender.send(packet).await?;
        Ok(())
    }

    pub async fn subscribe(&self) -> Result<broadcast::Receiver<E>> {
        Ok(self.event_receiver_sender.subscribe())
    }

    pub async fn send(&self, request: R) -> Result<R::Response> {
        let (sender, receiver) = oneshot::channel::<R::Response>();
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
            .send(IdmTransportPacket {
                id: req,
                channel: self.channel,
                form: IdmTransportPacketForm::Request.into(),
                data: request.encode()?,
            })
            .await?;

        let response = timeout(Duration::from_secs(IDM_REQUEST_TIMEOUT_SECS), receiver).await??;
        success.store(true, Ordering::Release);
        Ok(response)
    }

    async fn process(
        mut backend: Box<dyn IdmBackend>,
        event_sender: broadcast::Sender<E>,
        requests: RequestMap<R>,
        request_backend_sender: broadcast::Sender<(u64, R)>,
        _event_receiver: broadcast::Receiver<E>,
        mut receiver: Receiver<IdmTransportPacket>,
    ) -> Result<()> {
        loop {
            select! {
                x = backend.recv() => match x {
                    Ok(packet) => {
                        match packet.form() {
                            IdmTransportPacketForm::Event => {
                                if let Ok(event) = E::decode(&packet.data) {
                                    let _ = event_sender.send(event);
                                }
                            },

                            IdmTransportPacketForm::Request => {
                                if let Ok(request) = R::decode(&packet.data) {
                                    let _ = request_backend_sender.send((packet.id, request));
                                }
                            },

                            IdmTransportPacketForm::Response => {
                                let mut requests = requests.lock().await;
                                if let Some(sender) = requests.remove(&packet.id) {
                                    drop(requests);

                                    if let Ok(response) = R::Response::decode(&packet.data) {
                                        let _ = sender.send(response);
                                    }
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
