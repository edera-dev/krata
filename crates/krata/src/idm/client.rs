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
use bytes::{Buf, BufMut, BytesMut};
use log::{debug, error};
use nix::sys::termios::{cfmakeraw, tcgetattr, tcsetattr, SetArg};
use prost::Message;
use tokio::{
    fs::File,
    io::{AsyncReadExt, AsyncWriteExt},
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

type OneshotRequestMap<R> = Arc<Mutex<HashMap<u64, oneshot::Sender<<R as IdmRequest>::Response>>>>;
type StreamRequestMap<R> = Arc<Mutex<HashMap<u64, Sender<<R as IdmRequest>::Response>>>>;
type StreamRequestUpdateMap<R> = Arc<Mutex<HashMap<u64, Sender<R>>>>;
pub type IdmInternalClient = IdmClient<internal::Request, internal::Event>;

const IDM_PACKET_QUEUE_LEN: usize = 100;
const IDM_REQUEST_TIMEOUT_SECS: u64 = 10;
const IDM_PACKET_MAX_SIZE: usize = 20 * 1024 * 1024;

#[async_trait::async_trait]
pub trait IdmBackend: Send {
    async fn recv(&mut self) -> Result<Vec<IdmTransportPacket>>;
    async fn send(&mut self, packet: IdmTransportPacket) -> Result<()>;
}

pub struct IdmFileBackend {
    read: Arc<Mutex<File>>,
    read_buffer: BytesMut,
    write: Arc<Mutex<File>>,
}

impl IdmFileBackend {
    pub async fn new(read_file: File, write_file: File) -> Result<IdmFileBackend> {
        IdmFileBackend::set_raw_port(&read_file)?;
        IdmFileBackend::set_raw_port(&write_file)?;
        Ok(IdmFileBackend {
            read: Arc::new(Mutex::new(read_file)),
            read_buffer: BytesMut::new(),
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
    async fn recv(&mut self) -> Result<Vec<IdmTransportPacket>> {
        let mut data = vec![0; 8192];
        let mut first = true;
        'read_more: loop {
            let mut packets = Vec::new();
            if !first {
                if !packets.is_empty() {
                    return Ok(packets);
                }
                let size = self.read.lock().await.read(&mut data).await?;
                self.read_buffer.extend_from_slice(&data[0..size]);
            }
            first = false;
            loop {
                if self.read_buffer.len() < 6 {
                    continue 'read_more;
                }

                let b1 = self.read_buffer[0];
                let b2 = self.read_buffer[1];

                if b1 != 0xff || b2 != 0xff {
                    self.read_buffer.clear();
                    continue 'read_more;
                }

                let size = (self.read_buffer[2] as u32
                    | (self.read_buffer[3] as u32) << 8
                    | (self.read_buffer[4] as u32) << 16
                    | (self.read_buffer[5] as u32) << 24) as usize;
                let needed = size + 6;
                if self.read_buffer.len() < needed {
                    continue 'read_more;
                }

                let mut packet = self.read_buffer.split_to(needed);
                packet.advance(6);

                match IdmTransportPacket::decode(packet) {
                    Ok(packet) => {
                        packets.push(packet);
                    }
                    Err(error) => {
                        return Err(anyhow!("received invalid idm packet: {}", error));
                    }
                }

                if self.read_buffer.is_empty() {
                    break;
                }
            }
            return Ok(packets);
        }
    }

    async fn send(&mut self, packet: IdmTransportPacket) -> Result<()> {
        let mut file = self.write.lock().await;
        let length = packet.encoded_len();
        let mut buffer = BytesMut::with_capacity(6 + length);
        buffer.put_slice(&[0xff, 0xff]);
        buffer.put_u32_le(length as u32);
        packet.encode(&mut buffer)?;
        file.write_all(&buffer).await?;
        Ok(())
    }
}

#[derive(Clone)]
pub struct IdmClient<R: IdmRequest, E: IdmSerializable> {
    channel: u64,
    request_backend_sender: broadcast::Sender<(u64, R)>,
    request_stream_backend_sender: broadcast::Sender<IdmClientStreamResponseHandle<R>>,
    next_request_id: Arc<Mutex<u64>>,
    event_receiver_sender: broadcast::Sender<E>,
    tx_sender: Sender<IdmTransportPacket>,
    requests: OneshotRequestMap<R>,
    request_streams: StreamRequestMap<R>,
    task: Arc<JoinHandle<()>>,
}

impl<R: IdmRequest, E: IdmSerializable> Drop for IdmClient<R, E> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}

pub struct IdmClientStreamRequestHandle<R: IdmRequest, E: IdmSerializable> {
    pub id: u64,
    pub receiver: Receiver<R::Response>,
    pub client: IdmClient<R, E>,
}

impl<R: IdmRequest, E: IdmSerializable> IdmClientStreamRequestHandle<R, E> {
    pub async fn update(&self, request: R) -> Result<()> {
        self.client
            .tx_sender
            .send(IdmTransportPacket {
                id: self.id,
                channel: self.client.channel,
                form: IdmTransportPacketForm::StreamRequestUpdate.into(),
                data: request.encode()?,
            })
            .await?;
        Ok(())
    }
}

impl<R: IdmRequest, E: IdmSerializable> Drop for IdmClientStreamRequestHandle<R, E> {
    fn drop(&mut self) {
        let id = self.id;
        let client = self.client.clone();
        tokio::task::spawn(async move {
            let _ = client
                .tx_sender
                .send(IdmTransportPacket {
                    id,
                    channel: client.channel,
                    form: IdmTransportPacketForm::StreamRequestClosed.into(),
                    data: vec![],
                })
                .await;
        });
    }
}

#[derive(Clone)]
pub struct IdmClientStreamResponseHandle<R: IdmRequest> {
    pub initial: R,
    pub id: u64,
    channel: u64,
    tx_sender: Sender<IdmTransportPacket>,
    receiver: Arc<Mutex<Option<Receiver<R>>>>,
}

impl<R: IdmRequest> IdmClientStreamResponseHandle<R> {
    pub async fn respond(&self, response: R::Response) -> Result<()> {
        self.tx_sender
            .send(IdmTransportPacket {
                id: self.id,
                channel: self.channel,
                form: IdmTransportPacketForm::StreamResponseUpdate.into(),
                data: response.encode()?,
            })
            .await?;
        Ok(())
    }

    pub async fn take(&self) -> Result<Receiver<R>> {
        let mut guard = self.receiver.lock().await;
        let Some(receiver) = (*guard).take() else {
            return Err(anyhow!("request has already been claimed!"));
        };
        Ok(receiver)
    }
}

impl<R: IdmRequest> Drop for IdmClientStreamResponseHandle<R> {
    fn drop(&mut self) {
        if Arc::strong_count(&self.receiver) <= 1 {
            let id = self.id;
            let channel = self.channel;
            let tx_sender = self.tx_sender.clone();
            tokio::task::spawn(async move {
                let _ = tx_sender
                    .send(IdmTransportPacket {
                        id,
                        channel,
                        form: IdmTransportPacketForm::StreamResponseClosed.into(),
                        data: vec![],
                    })
                    .await;
            });
        }
    }
}

impl<R: IdmRequest, E: IdmSerializable> IdmClient<R, E> {
    pub async fn new(channel: u64, backend: Box<dyn IdmBackend>) -> Result<Self> {
        let requests = Arc::new(Mutex::new(HashMap::new()));
        let request_streams = Arc::new(Mutex::new(HashMap::new()));
        let request_update_streams = Arc::new(Mutex::new(HashMap::new()));
        let (event_sender, event_receiver) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (internal_request_backend_sender, _) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (internal_request_stream_backend_sender, _) = broadcast::channel(IDM_PACKET_QUEUE_LEN);
        let (tx_sender, tx_receiver) = mpsc::channel(IDM_PACKET_QUEUE_LEN);
        let backend_event_sender = event_sender.clone();
        let request_backend_sender = internal_request_backend_sender.clone();
        let request_stream_backend_sender = internal_request_stream_backend_sender.clone();
        let requests_for_client = requests.clone();
        let request_streams_for_client = request_streams.clone();
        let tx_sender_for_client = tx_sender.clone();
        let task = tokio::task::spawn(async move {
            if let Err(error) = IdmClient::process(
                backend,
                channel,
                tx_sender,
                backend_event_sender,
                requests,
                request_streams,
                request_update_streams,
                internal_request_backend_sender,
                internal_request_stream_backend_sender,
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
            request_stream_backend_sender,
            requests: requests_for_client,
            request_streams: request_streams_for_client,
            tx_sender: tx_sender_for_client,
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

    pub async fn request_streams(
        &self,
    ) -> Result<broadcast::Receiver<IdmClientStreamResponseHandle<R>>> {
        Ok(self.request_stream_backend_sender.subscribe())
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

    pub async fn send_stream(&self, request: R) -> Result<IdmClientStreamRequestHandle<R, E>> {
        let (sender, receiver) = mpsc::channel::<R::Response>(100);
        let req = {
            let mut guard = self.next_request_id.lock().await;
            let req = *guard;
            *guard = req.wrapping_add(1);
            req
        };
        let mut requests = self.request_streams.lock().await;
        requests.insert(req, sender);
        drop(requests);
        self.tx_sender
            .send(IdmTransportPacket {
                id: req,
                channel: self.channel,
                form: IdmTransportPacketForm::StreamRequest.into(),
                data: request.encode()?,
            })
            .await?;
        Ok(IdmClientStreamRequestHandle {
            id: req,
            receiver,
            client: self.clone(),
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn process(
        mut backend: Box<dyn IdmBackend>,
        channel: u64,
        tx_sender: Sender<IdmTransportPacket>,
        event_sender: broadcast::Sender<E>,
        requests: OneshotRequestMap<R>,
        request_streams: StreamRequestMap<R>,
        request_update_streams: StreamRequestUpdateMap<R>,
        request_backend_sender: broadcast::Sender<(u64, R)>,
        request_stream_backend_sender: broadcast::Sender<IdmClientStreamResponseHandle<R>>,
        _event_receiver: broadcast::Receiver<E>,
        mut receiver: Receiver<IdmTransportPacket>,
    ) -> Result<()> {
        loop {
            select! {
                x = backend.recv() => match x {
                    Ok(packets) => {
                        for packet in packets {
                         if packet.channel != channel {
                            continue;
                        }

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

                            IdmTransportPacketForm::StreamRequest => {
                                if let Ok(request) = R::decode(&packet.data) {
                                    let mut update_streams = request_update_streams.lock().await;
                                    let (sender, receiver) = mpsc::channel(100);
                                    update_streams.insert(packet.id, sender.clone());
                                    let handle = IdmClientStreamResponseHandle {
                                        initial: request,
                                        id: packet.id,
                                        channel,
                                        tx_sender: tx_sender.clone(),
                                        receiver: Arc::new(Mutex::new(Some(receiver))),
                                    };
                                    let _ = request_stream_backend_sender.send(handle);
                                }
                            }

                            IdmTransportPacketForm::StreamRequestUpdate => {
                                if let Ok(request) = R::decode(&packet.data) {
                                    let mut update_streams = request_update_streams.lock().await;
                                    if let Some(stream) = update_streams.get_mut(&packet.id) {
                                        let _ = stream.try_send(request);
                                    }
                                }
                            }

                            IdmTransportPacketForm::StreamRequestClosed => {
                                let mut update_streams = request_update_streams.lock().await;
                                update_streams.remove(&packet.id);
                            }

                            IdmTransportPacketForm::StreamResponseUpdate => {
                                let requests = request_streams.lock().await;
                                if let Some(sender) = requests.get(&packet.id) {
                                    if let Ok(response) = R::Response::decode(&packet.data) {
                                        let _ = sender.try_send(response);
                                    }
                                }
                            }

                            IdmTransportPacketForm::StreamResponseClosed => {
                                let mut requests = request_streams.lock().await;
                                requests.remove(&packet.id);
                            }

                            _ => {},
                        }
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
                        backend.send(packet.clone()).await?;
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
