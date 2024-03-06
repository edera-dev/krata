use std::{fmt::Display, str::FromStr};

use anyhow::anyhow;
use url::{Host, Url};

pub const KRATA_DEFAULT_TCP_PORT: u16 = 4350;
pub const KRATA_DEFAULT_TLS_PORT: u16 = 4353;

#[derive(Clone)]
pub enum ControlDialAddress {
    UnixSocket {
        path: String,
    },
    Tcp {
        host: String,
        port: u16,
    },
    Tls {
        host: String,
        port: u16,
        insecure: bool,
    },
}

impl FromStr for ControlDialAddress {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let url: Url = s.parse()?;

        let host = url.host().unwrap_or(Host::Domain("localhost")).to_string();

        match url.scheme() {
            "unix" => Ok(ControlDialAddress::UnixSocket {
                path: url.path().to_string(),
            }),

            "tcp" => {
                let port = url.port().unwrap_or(KRATA_DEFAULT_TCP_PORT);
                Ok(ControlDialAddress::Tcp { host, port })
            }

            "tls" | "tls-insecure" => {
                let insecure = url.scheme() == "tls-insecure";
                let port = url.port().unwrap_or(KRATA_DEFAULT_TLS_PORT);
                Ok(ControlDialAddress::Tls {
                    host,
                    port,
                    insecure,
                })
            }

            _ => Err(anyhow!("unknown control address scheme: {}", url.scheme())),
        }
    }
}

impl From<ControlDialAddress> for Url {
    fn from(val: ControlDialAddress) -> Self {
        match val {
            ControlDialAddress::UnixSocket { path } => {
                let mut url = Url::parse("unix:///").unwrap();
                url.set_path(&path);
                url
            }

            ControlDialAddress::Tcp { host, port } => {
                let mut url = Url::parse("tcp://").unwrap();
                url.set_host(Some(&host)).unwrap();
                if port != KRATA_DEFAULT_TCP_PORT {
                    url.set_port(Some(port)).unwrap();
                }
                url
            }

            ControlDialAddress::Tls {
                host,
                port,
                insecure,
            } => {
                let mut url = Url::parse("tls://").unwrap();
                if insecure {
                    url.set_scheme("tls-insecure").unwrap();
                }
                url.set_host(Some(&host)).unwrap();
                if port != KRATA_DEFAULT_TLS_PORT {
                    url.set_port(Some(port)).unwrap();
                }
                url
            }
        }
    }
}

impl Display for ControlDialAddress {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let url: Url = self.clone().into();
        write!(f, "{}", url)
    }
}
