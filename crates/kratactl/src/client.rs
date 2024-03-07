use anyhow::Result;
use krata::{control::control_service_client::ControlServiceClient, dial::ControlDialAddress};
use tokio::net::UnixStream;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint, Uri};
use tower::service_fn;

pub struct ControlClientProvider {}

impl ControlClientProvider {
    pub async fn dial(addr: ControlDialAddress) -> Result<ControlServiceClient<Channel>> {
        let channel = match addr {
            ControlDialAddress::UnixSocket { path } => {
                // This URL is not actually used but is required to be specified.
                Endpoint::try_from(format!("unix://localhost/{}", path))?
                    .connect_with_connector(service_fn(|uri: Uri| {
                        let path = uri.path().to_string();
                        UnixStream::connect(path)
                    }))
                    .await?
            }

            ControlDialAddress::Tcp { host, port } => {
                Endpoint::try_from(format!("http://{}:{}", host, port))?
                    .connect()
                    .await?
            }

            ControlDialAddress::Tls {
                host,
                port,
                insecure: _,
            } => {
                let tls_config = ClientTlsConfig::new().domain_name(&host);
                let address = format!("https://{}:{}", host, port);
                Channel::from_shared(address)?
                    .tls_config(tls_config)?
                    .connect()
                    .await?
            }
        };

        Ok(ControlServiceClient::new(channel))
    }
}
