use std::{collections::HashMap, sync::Arc};

use anyhow::{anyhow, Result};
use krata::{
    control::{Message, Request, RequestBox, Response},
    stream::{ConnectionStreams, StreamContext},
};
use log::{trace, warn};
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    net::{TcpStream, UnixStream},
    select,
    sync::{
        mpsc::{channel, Receiver, Sender},
        oneshot, Mutex,
    },
    task::JoinHandle,
};
use tokio_native_tls::TlsStream;
use tokio_stream::{wrappers::LinesStream, StreamExt};

const QUEUE_MAX_LEN: usize = 100;

pub struct KrataClientTransport {
    sender: Sender<Message>,
    receiver: Receiver<Message>,
    task: JoinHandle<()>,
}

impl Drop for KrataClientTransport {
    fn drop(&mut self) {
        self.task.abort();
    }
}

macro_rules! transport_new {
    ($name:ident, $stream:ty, $processor:ident) => {
        pub async fn $name(stream: $stream) -> Result<Self> {
            let (tx_sender, tx_receiver) = channel::<Message>(QUEUE_MAX_LEN);
            let (rx_sender, rx_receiver) = channel::<Message>(QUEUE_MAX_LEN);

            let task = tokio::task::spawn(async move {
                if let Err(error) =
                    KrataClientTransport::$processor(stream, rx_sender, tx_receiver).await
                {
                    warn!("failed to process krata transport messages: {}", error);
                }
            });

            Ok(Self {
                sender: tx_sender,
                receiver: rx_receiver,
                task,
            })
        }
    };
}

macro_rules! transport_processor {
    ($name:ident, $stream:ty) => {
        async fn $name(
            stream: $stream,
            rx_sender: Sender<Message>,
            mut tx_receiver: Receiver<Message>,
        ) -> Result<()> {
            let (read, mut write) = tokio::io::split(stream);
            let mut read = LinesStream::new(BufReader::new(read).lines());
            loop {
                select! {
                    x = tx_receiver.recv() => match x {
                        Some(message) => {
                            let mut line = serde_json::to_string(&message)?;
                            trace!("sending line '{}'", line);
                            line.push('\n');
                            write.write_all(line.as_bytes()).await?;
                        },

                        None => {
                            break;
                        }
                    },

                    x = read.next() => match x {
                        Some(Ok(line)) => {
                            let message = serde_json::from_str::<Message>(&line)?;
                            rx_sender.send(message).await?;
                        },

                        Some(Err(error)) => {
                            return Err(error.into());
                        },

                        None => {
                            break;
                        }
                    }
                };
            }
            Ok(())
        }
    };
}

impl KrataClientTransport {
    transport_new!(from_unix, UnixStream, process_unix_stream);
    transport_new!(from_tcp, TcpStream, process_tcp_stream);
    transport_new!(from_tls_tcp, TlsStream<TcpStream>, process_tls_tcp_stream);

    transport_processor!(process_unix_stream, UnixStream);
    transport_processor!(process_tcp_stream, TcpStream);
    transport_processor!(process_tls_tcp_stream, TlsStream<TcpStream>);
}

type RequestsMap = Arc<Mutex<HashMap<u64, oneshot::Sender<Response>>>>;

#[derive(Clone)]
pub struct KrataClient {
    tx_sender: Sender<Message>,
    next: Arc<Mutex<u64>>,
    streams: ConnectionStreams,
    requests: RequestsMap,
    task: Arc<JoinHandle<()>>,
}

impl KrataClient {
    pub async fn new(transport: KrataClientTransport) -> Result<Self> {
        let tx_sender = transport.sender.clone();
        let streams = ConnectionStreams::new(tx_sender.clone());
        let requests = Arc::new(Mutex::new(HashMap::new()));
        let task = {
            let requests = requests.clone();
            let streams = streams.clone();
            tokio::task::spawn(async move {
                if let Err(error) = KrataClient::process(transport, streams, requests).await {
                    warn!("failed to process krata client messages: {}", error);
                }
            })
        };

        Ok(Self {
            tx_sender,
            next: Arc::new(Mutex::new(0)),
            requests,
            streams,
            task: Arc::new(task),
        })
    }

    pub async fn send(&self, request: Request) -> Result<Response> {
        let id = {
            let mut next = self.next.lock().await;
            let id = *next;
            *next = id + 1;
            id
        };
        let (sender, receiver) = oneshot::channel();
        self.requests.lock().await.insert(id, sender);
        self.tx_sender
            .send(Message::Request(RequestBox { id, request }))
            .await?;
        let response = receiver.await?;
        if let Response::Error(error) = response {
            Err(anyhow!("krata error: {}", error.message))
        } else {
            Ok(response)
        }
    }

    pub async fn acquire(&self, stream: u64) -> Result<StreamContext> {
        self.streams.acquire(stream).await
    }

    async fn process(
        mut transport: KrataClientTransport,
        streams: ConnectionStreams,
        requests: RequestsMap,
    ) -> Result<()> {
        loop {
            let Some(message) = transport.receiver.recv().await else {
                break;
            };

            match message {
                Message::Request(_) => {
                    return Err(anyhow!("received request from service"));
                }

                Message::Response(resp) => {
                    let Some(sender) = requests.lock().await.remove(&resp.id) else {
                        continue;
                    };

                    let _ = sender.send(resp.response);
                }

                Message::StreamUpdated(updated) => {
                    streams.incoming(updated).await?;
                }
            }
        }
        Ok(())
    }
}

impl Drop for KrataClient {
    fn drop(&mut self) {
        if Arc::strong_count(&self.task) <= 1 {
            self.task.abort();
        }
    }
}
