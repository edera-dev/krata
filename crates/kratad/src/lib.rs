use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::Result;
use control::RuntimeControlService;
use db::GuestStore;
use event::{DaemonEventContext, DaemonEventGenerator};
use krata::{dial::ControlDialAddress, v1::control::control_service_server::ControlServiceServer};
use kratart::Runtime;
use log::info;
use reconcile::guest::GuestReconciler;
use tokio::{
    net::UnixListener,
    sync::mpsc::{channel, Sender},
    task::JoinHandle,
};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use uuid::Uuid;

pub mod control;
pub mod db;
pub mod event;
pub mod reconcile;

pub struct Daemon {
    store: String,
    runtime: Runtime,
    guests: GuestStore,
    events: DaemonEventContext,
    guest_reconciler_task: JoinHandle<()>,
    guest_reconciler_notify: Sender<Uuid>,
    generator_task: JoinHandle<()>,
}

const GUEST_RECONCILER_QUEUE_LEN: usize = 1000;

impl Daemon {
    pub async fn new(store: String, runtime: Runtime) -> Result<Self> {
        let guests_db_path = format!("{}/guests.db", store);
        let guests = GuestStore::open(&PathBuf::from(guests_db_path))?;
        let runtime_for_events = runtime.dupe().await?;
        let (guest_reconciler_notify, guest_reconciler_receiver) =
            channel::<Uuid>(GUEST_RECONCILER_QUEUE_LEN);
        let (events, generator) = DaemonEventGenerator::new(
            guests.clone(),
            guest_reconciler_notify.clone(),
            runtime_for_events,
        )
        .await?;
        let runtime_for_reconciler = runtime.dupe().await?;
        let guest_reconciler =
            GuestReconciler::new(guests.clone(), events.clone(), runtime_for_reconciler)?;
        Ok(Self {
            store,
            runtime,
            guests,
            events,
            guest_reconciler_task: guest_reconciler.launch(guest_reconciler_receiver).await?,
            guest_reconciler_notify,
            generator_task: generator.launch().await?,
        })
    }

    pub async fn listen(&mut self, addr: ControlDialAddress) -> Result<()> {
        let control_service = RuntimeControlService::new(
            self.events.clone(),
            self.runtime.clone(),
            self.guests.clone(),
            self.guest_reconciler_notify.clone(),
        );

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
        self.guest_reconciler_task.abort();
        self.generator_task.abort();
    }
}
