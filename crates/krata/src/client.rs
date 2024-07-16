#[cfg(unix)]
use crate::unix::HyperUnixConnector;
use crate::{dial::ControlDialAddress, v1::control::control_service_client::ControlServiceClient};
#[cfg(not(unix))]
use anyhow::anyhow;
use anyhow::Result;
use tonic::transport::{Channel, ClientTlsConfig, Endpoint};

pub struct ControlClientProvider {}

impl ControlClientProvider {
    pub async fn dial(addr: ControlDialAddress) -> Result<ControlServiceClient<Channel>> {
        let channel = match addr {
            ControlDialAddress::UnixSocket { path } => {
                #[cfg(not(unix))]
                return Err(anyhow!(
                    "unix sockets are not supported on this platform (path {})",
                    path
                ));
                #[cfg(unix)]
                ControlClientProvider::dial_unix_socket(path).await?
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

    #[cfg(unix)]
    async fn dial_unix_socket(path: String) -> Result<Channel> {
        // This URL is not actually used but is required to be specified.
        Ok(Endpoint::try_from(format!("unix://localhost/{}", path))?
            .connect_with_connector(HyperUnixConnector {})
            .await?)
    }
}
