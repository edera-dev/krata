use std::{net::SocketAddr, path::PathBuf, str::FromStr};

use anyhow::Result;
use control::RuntimeControlService;
use event::{DaemonEventContext, DaemonEventGenerator};
use krata::{control::control_service_server::ControlServiceServer, dial::ControlDialAddress};
use kratart::{launch::GuestLaunchRequest, Runtime};
use log::{info, warn};
use tab::Tab;
use tokio::{fs, net::UnixListener, task::JoinHandle};
use tokio_stream::wrappers::UnixListenerStream;
use tonic::transport::{Identity, Server, ServerTlsConfig};

pub mod control;
pub mod event;
pub mod tab;

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

    pub async fn load_guest_tab(&mut self) -> Result<()> {
        let tab_path = PathBuf::from(format!("{}/guests.yml", self.store));

        if !tab_path.exists() {
            return Ok(());
        }

        info!("loading guest tab");

        let tab_content = fs::read_to_string(tab_path).await?;
        let tab: Tab = serde_yaml::from_str(&tab_content)?;
        let running = self.runtime.list().await?;
        for (name, guest) in tab.guests {
            let existing = running
                .iter()
                .filter(|x| x.name.is_some())
                .find(|run| *run.name.as_ref().unwrap() == name);

            if let Some(existing) = existing {
                info!("guest {} is already running: {}", name, existing.uuid);
                continue;
            }

            let request = GuestLaunchRequest {
                name: Some(&name),
                image: &guest.image,
                vcpus: guest.cpus,
                mem: guest.mem,
                env: if guest.env.is_empty() {
                    None
                } else {
                    Some(
                        guest
                            .env
                            .iter()
                            .map(|(key, value)| format!("{}={}", key, value))
                            .collect::<Vec<String>>(),
                    )
                },
                run: if guest.run.is_empty() {
                    None
                } else {
                    Some(guest.run)
                },
                debug: false,
            };
            match self.runtime.launch(request).await {
                Err(error) => {
                    warn!("failed to launch guest {}: {}", name, error);
                }

                Ok(info) => {
                    info!("launched guest {}: {}", name, info.uuid);
                }
            }
        }
        info!("loaded guest tab");
        Ok(())
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
