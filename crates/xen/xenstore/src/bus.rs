use std::{
    collections::HashMap,
    ffi::CString,
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::fs::FileTypeExt,
    },
    sync::Arc,
};

use log::{debug, warn};
use tokio::{
    fs::{metadata, File},
    io::AsyncWriteExt,
    net::UnixStream,
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        oneshot::{self, channel as oneshot_channel},
        Mutex,
    },
    task::JoinHandle,
};

use crate::{
    error::{Error, Result},
    sys::{XsdMessageHeader, XSD_ERROR, XSD_UNWATCH, XSD_WATCH_EVENT},
};

const XEN_BUS_PATHS: &[&str] = &["/var/run/xenstored/socket", "/dev/xen/xenbus"];
const XEN_BUS_MAX_PAYLOAD_SIZE: usize = 4096;
const XEN_BUS_MAX_PACKET_SIZE: usize = XsdMessageHeader::SIZE + XEN_BUS_MAX_PAYLOAD_SIZE;

async fn find_bus_path() -> Option<(&'static str, bool)> {
    for path in XEN_BUS_PATHS {
        match metadata(path).await {
            Ok(metadata) => {
                return Some((path, metadata.file_type().is_socket()));
            }
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
    unwatch_sender: Sender<(u32, String)>,
    _rx_task: Arc<std::thread::JoinHandle<()>>,
}

impl XsdSocket {
    pub async fn open() -> Result<XsdSocket> {
        let (path, socket) = match find_bus_path().await {
            Some(path) => path,
            None => return Err(Error::BusNotFound),
        };

        let file = if socket {
            let stream = UnixStream::connect(path).await?;
            let stream = stream.into_std()?;
            stream.set_nonblocking(false)?;
            unsafe { File::from_raw_fd(stream.into_raw_fd()) }
        } else {
            File::options().read(true).write(true).open(path).await?
        };

        XsdSocket::from_handle(file).await
    }

    pub async fn from_handle(handle: File) -> Result<XsdSocket> {
        let replies: ReplyMap = Arc::new(Mutex::new(HashMap::new()));
        let watches: WatchMap = Arc::new(Mutex::new(HashMap::new()));

        let next_request_id = Arc::new(Mutex::new(0u32));

        let (rx_sender, rx_receiver) = channel::<XsdMessage>(10);
        let (tx_sender, tx_receiver) = channel::<XsdMessage>(10);
        let (unwatch_sender, unwatch_receiver) = channel::<(u32, String)>(1000);
        let read: std::fs::File = unsafe { std::fs::File::from_raw_fd(handle.as_raw_fd()) };

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

        let rx_task = std::thread::Builder::new()
            .name("xenstore-reader".to_string())
            .spawn(move || {
                let mut read = read;
                if let Err(error) = XsdSocketProcessor::process_rx(&mut read, rx_sender) {
                    debug!("failed to process xen store bus: {}", error);
                }
                std::mem::forget(read);
            })?;

        Ok(XsdSocket {
            tx_sender,
            replies,
            watches,
            next_request_id,
            next_watch_id: Arc::new(Mutex::new(0u32)),
            processor_task: Arc::new(processor_task),
            unwatch_sender,
            _rx_task: Arc::new(rx_task),
        })
    }

    pub async fn send_buf(&self, tx: u32, typ: u32, payload: &[u8]) -> Result<XsdMessage> {
        let req = {
            let mut guard = self.next_request_id.lock().await;
            let req = *guard;
            *guard = req.wrapping_add(1);
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

    pub async fn add_watch(&self) -> Result<(u32, Receiver<String>, Sender<(u32, String)>)> {
        let id = {
            let mut guard = self.next_watch_id.lock().await;
            let watch = *guard;
            *guard = watch.wrapping_add(1);
            watch
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
    unwatch_receiver: Receiver<(u32, String)>,
}

impl XsdSocketProcessor {
    fn process_rx(read: &mut std::fs::File, rx_sender: Sender<XsdMessage>) -> Result<()> {
        let mut header_buffer: Vec<u8> = vec![0u8; XsdMessageHeader::SIZE];
        let mut buffer: Vec<u8> = vec![0u8; XEN_BUS_MAX_PACKET_SIZE - XsdMessageHeader::SIZE];
        loop {
            let message = XsdSocketProcessor::read_message(&mut header_buffer, &mut buffer, read)?;
            rx_sender.blocking_send(message)?;
        }
    }

    fn read_message(
        header_buffer: &mut [u8],
        buffer: &mut [u8],
        read: &mut std::fs::File,
    ) -> Result<XsdMessage> {
        read.read_exact(header_buffer)?;
        let header = XsdMessageHeader::decode(header_buffer)?;
        if header.len as usize > buffer.len() {
            return Err(Error::InvalidBusData);
        }
        let payload_buffer = &mut buffer[0..header.len as usize];
        read.read_exact(payload_buffer)?;
        Ok(XsdMessage {
            header,
            payload: payload_buffer.to_vec(),
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
                    Some((id, path)) => {
                        let req = {
                            let mut guard = self.next_request_id.lock().await;
                            let req = *guard;
                            *guard = req.wrapping_add(1);
                            req
                        };

                        let mut payload = path.as_bytes().to_vec();
                        payload.push(0);
                        payload.extend_from_slice(id.to_string().as_bytes());
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
            }
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
        if Arc::strong_count(&self.processor_task) <= 1 {
            self.processor_task.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        sys::{XSD_READ, XSD_TRANSACTION_END, XSD_TRANSACTION_START, XSD_WATCH},
        XsdClient, XsdInterface, XsdWatchHandle,
    };
    use std::{net::Shutdown, os::fd::OwnedFd, time::Duration};
    use tokio::{io::AsyncReadExt, time::timeout};

    /* Test transport helpers. */

    struct PeerCleanup {
        stream: std::os::unix::net::UnixStream,
        processor: Arc<JoinHandle<()>>,
    }

    impl Drop for PeerCleanup {
        fn drop(&mut self) {
            // Unblock the real reader even when a test assertion or deadline fails.
            let _ = self.stream.shutdown(Shutdown::Both);
            self.processor.abort();
        }
    }

    async fn receive(peer: &mut UnixStream) -> XsdMessage {
        let mut bytes = [0; XsdMessageHeader::SIZE];
        peer.read_exact(&mut bytes).await.unwrap();
        let header = XsdMessageHeader::decode(&bytes).unwrap();
        assert!(header.len as usize <= XEN_BUS_MAX_PAYLOAD_SIZE);
        let mut payload = vec![0; header.len as usize];
        peer.read_exact(&mut payload).await.unwrap();
        XsdMessage { header, payload }
    }

    async fn send(peer: &mut UnixStream, header: XsdMessageHeader, payload: &[u8]) {
        let header = XsdMessageHeader {
            len: payload.len() as u32,
            ..header
        };
        peer.write_all(&header.encode().unwrap()).await.unwrap();
        peer.write_all(payload).await.unwrap();
    }

    async fn bind(peer: &mut UnixStream, client: &XsdClient, watch: &XsdWatchHandle) -> XsdMessage {
        let (result, message) = tokio::join!(client.bind_watch(watch), async {
            let message = receive(peer).await;
            assert_eq!(message.header.typ, XSD_WATCH);
            assert_eq!(message.header.tx, 0);
            assert_eq!(
                message.payload,
                format!("{}\0{}\0", watch.path, watch.id).as_bytes()
            );
            send(peer, message.header, b"OK\0").await;
            message
        });
        result.unwrap();
        message
    }

    /* End test transport helpers. */

    #[tokio::test]
    async fn unwatch_preserves_watch_identity() {
        let (connection, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let file = std::fs::File::from(OwnedFd::from(connection));
        let socket = XsdSocket::from_handle(File::from_std(file)).await.unwrap();
        let reader = socket._rx_task.clone();
        let cleanup = PeerCleanup {
            stream: peer.try_clone().unwrap(),
            processor: socket.processor_task.clone(),
        };
        peer.set_nonblocking(true).unwrap();
        let mut peer = UnixStream::from_std(peer).unwrap();
        let client = XsdClient { socket };

        // Keep cleanup outside the scenario so panics and stalled replies also release the reader.
        let mut scenario = tokio::spawn(async move {
            let removed = client
                .create_watch("/local/domain/42/device/vbd/0/state")
                .await
                .unwrap();
            let mut surviving = client
                .create_watch("/local/domain/42/device/vif/0/state")
                .await
                .unwrap();
            let registered = bind(&mut peer, &client, &removed).await;
            let other = bind(&mut peer, &client, &surviving).await;
            assert_ne!(registered.payload, other.payload);
            assert_eq!(registered.header.req, 0);
            assert_eq!(other.header.req, 1);

            drop(removed);
            let unwatch = receive(&mut peer).await;
            assert_eq!(unwatch.header.typ, XSD_UNWATCH);
            assert_eq!(unwatch.header.req, 2);
            assert_eq!(unwatch.header.tx, 0);
            assert_eq!(unwatch.header.len, registered.header.len);
            assert_eq!(unwatch.payload.last(), Some(&0));
            assert_eq!(unwatch.payload.iter().filter(|&&byte| byte == 0).count(), 2);
            send(&mut peer, unwatch.header, b"OK\0").await;

            let (transaction, ()) = tokio::join!(client.transaction(), async {
                let request = receive(&mut peer).await;
                assert_eq!(request.header.typ, XSD_TRANSACTION_START);
                assert_eq!(request.header.req, 3);
                assert_eq!(request.header.tx, 0);
                assert_eq!(request.payload, b"\0");
                send(&mut peer, request.header, b"71\0").await;
            });
            let transaction = transaction.unwrap();
            let path = surviving.path.clone();
            let event_path = format!("{path}/child");
            let event = format!("{event_path}\0{}\0", surviving.id);

            let (ordinary, transactional, ()) = tokio::join!(
                XsdInterface::read_string(&client, &path),
                transaction.read_string(&path),
                async {
                    let first = receive(&mut peer).await;
                    let second = receive(&mut peer).await;
                    let mut identities = vec![first.header.req, second.header.req];
                    identities.sort_unstable();
                    assert_eq!(identities, [4, 5]);
                    assert_ne!(first.header.tx, second.header.tx);
                    // Reverse replies and interleave an unsolicited event to check routing.
                    for request in [&second, &first] {
                        assert_eq!(request.header.typ, XSD_READ);
                        assert_eq!(request.payload, format!("{path}\0").as_bytes());
                        let value: &[u8] = match request.header.tx {
                            0 => b"ordinary",
                            71 => b"transactional",
                            tx => panic!("unexpected transaction {tx}"),
                        };
                        send(&mut peer, request.header, value).await;
                        if request.header.req == second.header.req {
                            send(
                                &mut peer,
                                XsdMessageHeader {
                                    typ: XSD_WATCH_EVENT,
                                    req: 0,
                                    tx: 0,
                                    len: 0,
                                },
                                event.as_bytes(),
                            )
                            .await;
                        }
                    }
                }
            );
            assert_eq!(ordinary.unwrap().as_deref(), Some("ordinary"));
            assert_eq!(transactional.unwrap().as_deref(), Some("transactional"));
            assert_eq!(
                surviving.receiver.recv().await.as_deref(),
                Some(event_path.as_str())
            );

            let (committed, ()) = tokio::join!(transaction.commit(), async {
                let request = receive(&mut peer).await;
                assert_eq!(request.header.typ, XSD_TRANSACTION_END);
                assert_eq!(request.header.req, 6);
                assert_eq!(request.header.tx, 71);
                assert_eq!(request.payload, b"T\0");
                send(&mut peer, request.header, b"OK\0").await;
            });
            assert!(committed.unwrap());
            (registered.payload, unwatch.payload)
        });

        let result = timeout(Duration::from_secs(5), &mut scenario).await;
        scenario.abort();
        drop(cleanup);
        timeout(Duration::from_secs(5), async {
            while !reader.is_finished() {
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("xenstore reader did not stop after peer shutdown");
        Arc::try_unwrap(reader)
            .expect("reader still referenced")
            .join()
            .unwrap();

        let (registered, unwatch) = result
            .expect("wire scenario timed out")
            .expect("wire scenario panicked");
        assert_eq!(
            unwatch, registered,
            "UNWATCH must repeat WATCH path then token"
        );
    }
}
