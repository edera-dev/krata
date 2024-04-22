use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::{anyhow, Result};
use console::{DaemonConsole, DaemonConsoleHandle};
use control::DaemonControlService;
use db::GuestStore;
use event::{DaemonEventContext, DaemonEventGenerator};
use glt::GuestLookupTable;
use idm::{DaemonIdm, DaemonIdmHandle};
use krata::{dial::ControlDialAddress, v1::control::control_service_server::ControlServiceServer};
use krataoci::{packer::service::OciPackerService, registry::OciPlatform};
use kratart::Runtime;
use log::info;
use reconcile::guest::GuestReconciler;
use tokio::{
    fs,
    net::UnixListener,
    sync::mpsc::{channel, Sender},
    task::JoinHandle,
};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Identity, Server, ServerTlsConfig};
use uuid::Uuid;

pub mod command;
pub mod console;
pub mod control;
pub mod db;
pub mod event;
pub mod glt;
pub mod idm;
pub mod metrics;
pub mod oci;
pub mod reconcile;

pub struct Daemon {
    store: String,
    glt: GuestLookupTable,
    guests: GuestStore,
    events: DaemonEventContext,
    guest_reconciler_task: JoinHandle<()>,
    guest_reconciler_notify: Sender<Uuid>,
    generator_task: JoinHandle<()>,
    idm: DaemonIdmHandle,
    console: DaemonConsoleHandle,
    packer: OciPackerService,
}

const GUEST_RECONCILER_QUEUE_LEN: usize = 1000;

impl Daemon {
    pub async fn new(store: String) -> Result<Self> {
        let mut image_cache_dir = PathBuf::from(store.clone());
        image_cache_dir.push("cache");
        image_cache_dir.push("image");
        fs::create_dir_all(&image_cache_dir).await?;

        let mut host_uuid_path = PathBuf::from(store.clone());
        host_uuid_path.push("host.uuid");
        let host_uuid = if host_uuid_path.is_file() {
            let content = fs::read_to_string(&host_uuid_path).await?;
            Uuid::from_str(content.trim()).ok()
        } else {
            None
        };

        let host_uuid = if let Some(host_uuid) = host_uuid {
            host_uuid
        } else {
            let generated = Uuid::new_v4();
            let mut string = generated.to_string();
            string.push('\n');
            fs::write(&host_uuid_path, string).await?;
            generated
        };

        let initrd_path = detect_guest_file(&store, "initrd")?;
        let kernel_path = detect_guest_file(&store, "kernel")?;

        let packer = OciPackerService::new(None, &image_cache_dir, OciPlatform::current()).await?;
        let runtime = Runtime::new().await?;
        let glt = GuestLookupTable::new(0, host_uuid);
        let guests_db_path = format!("{}/guests.db", store);
        let guests = GuestStore::open(&PathBuf::from(guests_db_path))?;
        let (guest_reconciler_notify, guest_reconciler_receiver) =
            channel::<Uuid>(GUEST_RECONCILER_QUEUE_LEN);
        let idm = DaemonIdm::new(glt.clone()).await?;
        let idm = idm.launch().await?;
        let console = DaemonConsole::new(glt.clone()).await?;
        let console = console.launch().await?;
        let (events, generator) =
            DaemonEventGenerator::new(guests.clone(), guest_reconciler_notify.clone(), idm.clone())
                .await?;
        let runtime_for_reconciler = runtime.dupe().await?;
        let guest_reconciler = GuestReconciler::new(
            glt.clone(),
            guests.clone(),
            events.clone(),
            runtime_for_reconciler,
            packer.clone(),
            guest_reconciler_notify.clone(),
            kernel_path,
            initrd_path,
        )?;

        let guest_reconciler_task = guest_reconciler.launch(guest_reconciler_receiver).await?;
        let generator_task = generator.launch().await?;

        Ok(Self {
            store,
            glt,
            guests,
            events,
            guest_reconciler_task,
            guest_reconciler_notify,
            generator_task,
            idm,
            console,
            packer,
        })
    }

    pub async fn listen(&mut self, addr: ControlDialAddress) -> Result<()> {
        let control_service = DaemonControlService::new(
            self.glt.clone(),
            self.events.clone(),
            self.console.clone(),
            self.idm.clone(),
            self.guests.clone(),
            self.guest_reconciler_notify.clone(),
            self.packer.clone(),
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
                    fs::remove_file(&path).await?;
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

fn detect_guest_file(store: &str, name: &str) -> Result<PathBuf> {
    let mut path = PathBuf::from(format!("{}/guest/{}", store, name));
    if path.is_file() {
        return Ok(path);
    }

    path = PathBuf::from(format!("/usr/share/krata/guest/{}", name));
    if path.is_file() {
        return Ok(path);
    }
    Err(anyhow!("unable to find required guest file: {}", name))
}
