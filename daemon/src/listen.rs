use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{anyhow, Result};
use krata::control::{ErrorResponse, Message, Request, RequestBox, Response, ResponseBox};
use log::trace;
use log::warn;
use tokio::sync::Mutex;
use tokio::{
    io::{AsyncBufReadExt, AsyncWriteExt, BufReader},
    select,
    sync::mpsc::{channel, Receiver, Sender},
};
use tokio_listener::{Connection, Listener, SomeSocketAddrClonable};
use tokio_stream::{wrappers::LinesStream, StreamExt};

use crate::runtime::Runtime;
use krata::stream::ConnectionStreams;

const QUEUE_MAX_LEN: usize = 100;

#[async_trait::async_trait]
pub trait DaemonRequestHandler: Send + Sync {
    fn accepts(&self, request: &Request) -> bool;
    async fn handle(
        &self,
        streams: ConnectionStreams,
        runtime: Runtime,
        request: Request,
    ) -> Result<Response>;
}

#[derive(Clone)]
pub struct DaemonRequestHandlers {
    runtime: Runtime,
    handlers: Arc<Vec<Box<dyn DaemonRequestHandler>>>,
}

impl DaemonRequestHandlers {
    pub fn new(runtime: Runtime, handlers: Vec<Box<dyn DaemonRequestHandler>>) -> Self {
        DaemonRequestHandlers {
            runtime,
            handlers: Arc::new(handlers),
        }
    }

    async fn dispatch(&self, streams: ConnectionStreams, request: Request) -> Result<Response> {
        for handler in self.handlers.iter() {
            if handler.accepts(&request) {
                return handler.handle(streams, self.runtime.clone(), request).await;
            }
        }
        Err(anyhow!("daemon cannot handle that request"))
    }
}

pub struct DaemonListener {
    listener: Listener,
    handlers: DaemonRequestHandlers,
    connections: Arc<Mutex<HashMap<u64, DaemonConnection>>>,
    next: Arc<Mutex<u64>>,
}

impl DaemonListener {
    pub fn new(listener: Listener, handlers: DaemonRequestHandlers) -> DaemonListener {
        DaemonListener {
            listener,
            handlers,
            connections: Arc::new(Mutex::new(HashMap::new())),
            next: Arc::new(Mutex::new(0)),
        }
    }

    pub async fn handle(&mut self) -> Result<()> {
        loop {
            let (connection, addr) = self.listener.accept().await?;
            let connection =
                DaemonConnection::new(connection, addr.clonable(), self.handlers.clone()).await?;
            let id = {
                let mut next = self.next.lock().await;
                let id = *next;
                *next = id + 1;
                id
            };
            trace!("new connection from {}", connection.addr);
            let tx_channel = connection.tx_sender.clone();
            let addr = connection.addr.clone();
            self.connections.lock().await.insert(id, connection);
            let connections_for_close = self.connections.clone();
            tokio::task::spawn(async move {
                tx_channel.closed().await;
                trace!("connection from {} closed", addr);
                connections_for_close.lock().await.remove(&id);
            });
        }
    }
}

#[derive(Clone)]
pub struct DaemonConnection {
    tx_sender: Sender<Message>,
    addr: SomeSocketAddrClonable,
    handlers: DaemonRequestHandlers,
    streams: ConnectionStreams,
}

impl DaemonConnection {
    pub async fn new(
        connection: Connection,
        addr: SomeSocketAddrClonable,
        handlers: DaemonRequestHandlers,
    ) -> Result<Self> {
        let (tx_sender, tx_receiver) = channel::<Message>(QUEUE_MAX_LEN);
        let streams_tx_sender = tx_sender.clone();
        let instance = DaemonConnection {
            tx_sender,
            addr,
            handlers,
            streams: ConnectionStreams::new(streams_tx_sender),
        };

        {
            let mut instance = instance.clone();
            tokio::task::spawn(async move {
                if let Err(error) = instance.process(tx_receiver, connection).await {
                    warn!(
                        "failed to process daemon connection for {}: {}",
                        instance.addr, error
                    );
                }
            });
        }

        Ok(instance)
    }

    async fn process(
        &mut self,
        mut tx_receiver: Receiver<Message>,
        connection: Connection,
    ) -> Result<()> {
        let (read, mut write) = tokio::io::split(connection);
        let mut read = LinesStream::new(BufReader::new(read).lines());

        loop {
            select! {
                x = read.next() => match x {
                    Some(Ok(line)) => {
                        let message: Message = serde_json::from_str(&line)?;
                        trace!("received message '{}' from {}", serde_json::to_string(&message)?, self.addr);
                        let mut context = self.clone();
                        tokio::task::spawn(async move {
                            if let Err(error) = context.handle_message(&message).await {
                                let line = serde_json::to_string(&message).unwrap_or("<invalid>".to_string());
                                warn!("failed to handle message '{}' from {}: {}", line, context.addr, error);
                            }
                        });
                    },

                    Some(Err(error)) => {
                        return Err(error.into());
                    },

                    None => {
                        break;
                    }
                },

                x = tx_receiver.recv() => match x {
                    Some(message) => {
                        if let Message::StreamUpdated(ref update) = message {
                            self.streams.outgoing(update).await?;
                        }
                        let mut line = serde_json::to_string(&message)?;
                        trace!("sending message '{}' to {}", line, self.addr);
                        line.push('\n');
                        write.write_all(line.as_bytes()).await?;
                    },
                    None => {
                        break;
                    }
                }
            };
        }
        Ok(())
    }

    async fn handle_message(&mut self, message: &Message) -> Result<()> {
        match message {
            Message::Request(req) => {
                self.handle_request(req.clone()).await?;
            }

            Message::Response(_) => {
                return Err(anyhow!(
                    "received a response message from client {}, but this is the daemon",
                    self.addr
                ));
            }

            Message::StreamUpdated(updated) => {
                self.streams.incoming(updated.clone()).await?;
            }
        }
        Ok(())
    }

    async fn handle_request(&mut self, req: RequestBox) -> Result<()> {
        let id = req.id;
        let response = self
            .handlers
            .dispatch(self.streams.clone(), req.request)
            .await
            .map_err(|error| {
                Response::Error(ErrorResponse {
                    message: error.to_string(),
                })
            });
        let response = if let Err(response) = response {
            response
        } else {
            response.unwrap()
        };
        let resp = ResponseBox { id, response };
        self.tx_sender.send(Message::Response(resp)).await?;
        Ok(())
    }
}
