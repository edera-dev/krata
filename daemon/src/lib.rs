use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::Result;
use control::RuntimeControlService;
use event::{DaemonEventContext, DaemonEventGenerator};
use krata::{control::control_service_server::ControlServiceServer, dial::ControlDialAddress};
use log::info;
use runtime::Runtime;
use tokio::{net::UnixListener, task::JoinHandle};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Identity, Server, ServerTlsConfig};

pub mod control;
pub mod event;
pub mod runtime;

pub struct Daemon {
    store: String,
    runtime: Runtime,
    events: DaemonEventContext,
    task: JoinHandle<()>,
}

impl Daemon {
    pub async fn new(store: String, runtime: Runtime) -> Result<Self> {
        let runtime_for_events = runtime.dupe().await?;
        let (events, generator) = DaemonEventGenerator::new(runtime_for_events).await?;
        Ok(Self {
            store,
            runtime,
            events,
            task: generator.launch().await?,
        })
    }

    pub async fn listen(&mut self, addr: ControlDialAddress) -> Result<()> {
        let control_service = RuntimeControlService::new(self.events.clone(), self.runtime.clone());

        let mut server = Server::builder();

        if let ControlDialAddress::Tls {
            host: _,
            port: _,
            insecure,
        } = &addr
        {
            let mut tls_config = ServerTlsConfig::new();
            if !insecure {
                let certificate_path = format!("{}/tls/daemon.pem", self.store);
                let key_path = format!("{}/tls/daemon.key", self.store);
                tls_config = tls_config.identity(Identity::from_pem(certificate_path, key_path));
            }
            server = server.tls_config(tls_config)?;
        }

        let server = server.add_service(ControlServiceServer::new(control_service));
        info!("listening on address {}", addr);
        match addr {
            ControlDialAddress::UnixSocket { path } => {
                let path = PathBuf::from(path);
                if path.exists() {
                    tokio::fs::remove_file(&path).await?;
                }
                let listener = UnixListener::bind(path)?;
                let stream = UnixListenerStream::new(listener);
                server.serve_with_incoming(stream).await?;
            }

            ControlDialAddress::Tcp { host, port } => {
                let address = format!("{}:{}", host, port);
                server.serve(SocketAddr::from_str(&address)?).await?;
            }

            ControlDialAddress::Tls {
                host,
                port,
                insecure: _,
            } => {
                let address = format!("{}:{}", host, port);
                server.serve(SocketAddr::from_str(&address)?).await?;
            }
        }
        Ok(())
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.task.abort();
    }
}
