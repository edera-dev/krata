use std::{collections::HashMap, ffi::CString, io::ErrorKind, sync::Arc, time::Duration};

use libc::O_NONBLOCK;
use log::warn;
use tokio::{
    fs::{metadata, File},
    io::{unix::AsyncFd, AsyncReadExt, AsyncWriteExt},
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        oneshot::{self, channel as oneshot_channel},
        Mutex,
    },
    task::JoinHandle,
    time::timeout,
};

use crate::{
    error::{Error, Result},
    sys::{XsdMessageHeader, XSD_ERROR, XSD_UNWATCH, XSD_WATCH_EVENT},
};

const XEN_BUS_PATHS: &[&str] = &["/dev/xen/xenbus"];
const XEN_BUS_MAX_PAYLOAD_SIZE: usize = 4096;
const XEN_BUS_MAX_PACKET_SIZE: usize = XsdMessageHeader::SIZE + XEN_BUS_MAX_PAYLOAD_SIZE;

async fn find_bus_path() -> Option<&'static str> {
    for path in XEN_BUS_PATHS {
        match metadata(path).await {
            Ok(_) => return Some(path),
            Err(_) => continue,
        }
    }
    None
}

struct WatchState {
    sender: Sender<String>,
}

struct ReplyState {
    sender: oneshot::Sender<XsdMessage>,
}

type ReplyMap = Arc<Mutex<HashMap<u32, ReplyState>>>;
type WatchMap = Arc<Mutex<HashMap<u32, WatchState>>>;

#[derive(Clone)]
pub struct XsdSocket {
    tx_sender: Sender<XsdMessage>,
    replies: ReplyMap,
    watches: WatchMap,
    next_request_id: Arc<Mutex<u32>>,
    next_watch_id: Arc<Mutex<u32>>,
    processor_task: Arc<JoinHandle<()>>,
    rx_task: Arc<JoinHandle<()>>,
    unwatch_sender: Sender<u32>,
}

impl XsdSocket {
    pub async fn open() -> Result<XsdSocket> {
        let path = match find_bus_path().await {
            Some(path) => path,
            None => return Err(Error::BusNotFound),
        };

        let file = File::options()
            .read(true)
            .write(true)
            .custom_flags(O_NONBLOCK)
            .open(path)
            .await?;
        XsdSocket::from_handle(file).await
    }

    pub async fn from_handle(handle: File) -> Result<XsdSocket> {
        let replies: ReplyMap = Arc::new(Mutex::new(HashMap::new()));
        let watches: WatchMap = Arc::new(Mutex::new(HashMap::new()));

        let next_request_id = Arc::new(Mutex::new(0u32));

        let (rx_sender, rx_receiver) = channel::<XsdMessage>(10);
        let (tx_sender, tx_receiver) = channel::<XsdMessage>(10);
        let (unwatch_sender, unwatch_receiver) = channel::<u32>(1000);
        let read: File = handle.try_clone().await?;

        let mut processor = XsdSocketProcessor {
            handle,
            replies: replies.clone(),
            watches: watches.clone(),
            next_request_id: next_request_id.clone(),
            tx_receiver,
            rx_receiver,
            unwatch_receiver,
        };

        let processor_task = tokio::task::spawn(async move {
            if let Err(error) = processor.process().await {
                warn!("failed to process xen store messages: {}", error);
            }
        });

        let rx_task = tokio::task::spawn(async move {
            if let Err(error) = XsdSocketProcessor::process_rx(read, rx_sender).await {
                warn!("failed to process xen store responses: {}", error);
            }
        });

        Ok(XsdSocket {
            tx_sender,
            replies,
            watches,
            next_request_id,
            next_watch_id: Arc::new(Mutex::new(0u32)),
            processor_task: Arc::new(processor_task),
            rx_task: Arc::new(rx_task),
            unwatch_sender,
        })
    }

    pub async fn send_buf(&self, tx: u32, typ: u32, payload: &[u8]) -> Result<XsdMessage> {
        let req = {
            let mut guard = self.next_request_id.lock().await;
            let req = *guard;
            *guard = req + 1;
            req
        };
        let (sender, receiver) = oneshot_channel::<XsdMessage>();
        self.replies.lock().await.insert(req, ReplyState { sender });

        let header = XsdMessageHeader {
            typ,
            req,
            tx,
            len: payload.len() as u32,
        };
        let message = XsdMessage {
            header,
            payload: payload.to_vec(),
        };
        if let Err(error) = self.tx_sender.try_send(message) {
            return Err(error.into());
        }
        let reply = receiver.await?;
        if reply.header.typ == XSD_ERROR {
            let error = CString::from_vec_with_nul(reply.payload)?;
            return Err(Error::ResponseError(error.into_string()?));
        }
        Ok(reply)
    }

    pub async fn send(&self, tx: u32, typ: u32, payload: &[&str]) -> Result<XsdMessage> {
        let mut buf: Vec<u8> = Vec::new();
        for item in payload {
            buf.extend_from_slice(item.as_bytes());
            buf.push(0);
        }
        self.send_buf(tx, typ, &buf).await
    }

    pub async fn add_watch(&self) -> Result<(u32, Receiver<String>, Sender<u32>)> {
        let id = {
            let mut guard = self.next_watch_id.lock().await;
            let req = *guard;
            *guard = req + 1;
            req
        };
        let (sender, receiver) = channel(10);
        self.watches.lock().await.insert(id, WatchState { sender });
        Ok((id, receiver, self.unwatch_sender.clone()))
    }
}

struct XsdSocketProcessor {
    handle: File,
    replies: ReplyMap,
    watches: WatchMap,
    next_request_id: Arc<Mutex<u32>>,
    tx_receiver: Receiver<XsdMessage>,
    rx_receiver: Receiver<XsdMessage>,
    unwatch_receiver: Receiver<u32>,
}

impl XsdSocketProcessor {
    async fn process_rx(read: File, rx_sender: Sender<XsdMessage>) -> Result<()> {
        let mut buffer: Vec<u8> = vec![0u8; XEN_BUS_MAX_PACKET_SIZE];
        let mut fd = AsyncFd::new(read)?;
        loop {
            select! {
                x = fd.readable_mut() => match x {
                    Ok(mut guard) => {
                        let future = XsdSocketProcessor::read_message(&mut buffer, guard.get_inner_mut());
                        if let Ok(message) = timeout(Duration::from_secs(1), future).await {
                            rx_sender.send(message?).await?;
                        }
                    },

                    Err(error) => {
                        return Err(error.into());
                    }
                },

                _ = rx_sender.closed() => {
                    break;
                }
            };
        }
        Ok(())
    }

    async fn read_message(buffer: &mut [u8], read: &mut File) -> Result<XsdMessage> {
        let size = loop {
            match read.read(buffer).await {
                Ok(size) => break size,
                Err(error) => {
                    if error.kind() == ErrorKind::WouldBlock {
                        tokio::task::yield_now().await;
                        continue;
                    }
                    return Err(error.into());
                }
            };
        };

        if size < XsdMessageHeader::SIZE {
            return Err(Error::InvalidBusData);
        }

        let header = XsdMessageHeader::decode(&buffer[0..XsdMessageHeader::SIZE])?;
        if size < XsdMessageHeader::SIZE + header.len as usize {
            return Err(Error::InvalidBusData);
        }
        let payload =
            &mut buffer[XsdMessageHeader::SIZE..XsdMessageHeader::SIZE + header.len as usize];
        Ok(XsdMessage {
            header,
            payload: payload.to_vec(),
        })
    }

    async fn process(&mut self) -> Result<()> {
        loop {
            select! {
                x = self.tx_receiver.recv() => match x {
                    Some(message) => {
                        let mut composed: Vec<u8> = Vec::new();
                        message.header.encode_to(&mut composed)?;
                        composed.extend_from_slice(&message.payload);
                        self.handle.write_all(&composed).await?;
                    }

                    None => {
                        break;
                    }
                },

                x = self.rx_receiver.recv() => match x {
                    Some(message) => {
                        if message.header.typ == XSD_WATCH_EVENT && message.header.req == 0 && message.header.tx == 0 {
                            let strings = message.parse_string_vec()?;
                            let Some(path) = strings.first() else {
                                return Ok(());
                            };
                            let Some(token) = strings.get(1) else {
                                return Ok(());
                            };

                            let Ok(id) = token.parse::<u32>() else {
                                return Ok(());
                            };

                            if let Some(state) = self.watches.lock().await.get(&id) {
                                let _ = state.sender.try_send(path.clone());
                            }
                        } else if let Some(state) = self.replies.lock().await.remove(&message.header.req) {
                            let _ = state.sender.send(message);
                        }
                    }

                    None => {
                        break;
                    }
                },

                x = self.unwatch_receiver.recv() => match x {
                    Some(id) => {
                        let req = {
                            let mut guard = self.next_request_id.lock().await;
                            let req = *guard;
                            *guard = req + 1;
                            req
                        };

                        let mut payload = id.to_string().as_bytes().to_vec();
                        payload.push(0);
                        let header = XsdMessageHeader {
                            typ: XSD_UNWATCH,
                            req,
                            tx: 0,
                            len: payload.len() as u32,
                        };
                        let mut data = header.encode()?;
                        data.extend_from_slice(&payload);
                        self.handle.write_all(&data).await?;
                    },

                    None => {
                        break;
                    }
                }
            };
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct XsdMessage {
    pub header: XsdMessageHeader,
    pub payload: Vec<u8>,
}

impl XsdMessage {
    pub fn parse_string(&self) -> Result<String> {
        Ok(CString::from_vec_with_nul(self.payload.clone())?.into_string()?)
    }

    pub fn parse_string_vec(&self) -> Result<Vec<String>> {
        let mut strings: Vec<String> = Vec::new();
        let mut buffer: Vec<u8> = Vec::new();
        for b in &self.payload {
            if *b == 0 {
                let string = String::from_utf8(buffer.clone())?;
                strings.push(string);
                buffer.clear();
                continue;
            }
            buffer.push(*b);
        }
        Ok(strings)
    }

    pub fn parse_bool(&self) -> Result<bool> {
        Ok(true)
    }
}

impl Drop for XsdSocket {
    fn drop(&mut self) {
        if Arc::strong_count(&self.rx_task) <= 1 {
            self.rx_task.abort();
        }

        if Arc::strong_count(&self.processor_task) <= 1 {
            self.processor_task.abort();
        }
    }
}
