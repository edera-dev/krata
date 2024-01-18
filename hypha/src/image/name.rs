use crate::error::Result;
use std::fmt;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageName {
    pub hostname: String,
    pub port: Option<u16>,
    pub name: String,
    pub reference: String,
}

impl fmt::Display for ImageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(port) = self.port {
            write!(
                f,
                "{}:{}/{}:{}",
                self.hostname, port, self.name, self.reference
            )
        } else {
            write!(f, "{}/{}:{}", self.hostname, self.name, self.reference)
        }
    }
}

impl Default for ImageName {
    fn default() -> Self {
        Self::parse(&format!("{}", uuid::Uuid::new_v4().as_hyphenated()))
            .expect("UUID hyphenated must be valid name")
    }
}

impl ImageName {
    pub fn parse(name: &str) -> Result<Self> {
        let (hostname, name) = name
            .split_once('/')
            .unwrap_or(("registry-1.docker.io", name));
        let (hostname, port) = if let Some((hostname, port)) = hostname.split_once(':') {
            (hostname, Some(str::parse(port)?))
        } else {
            (hostname, None)
        };
        let (name, reference) = name.split_once(':').unwrap_or((name, "latest"));
        Ok(ImageName {
            hostname: hostname.to_string(),
            port,
            name: name.to_string(),
            reference: reference.to_string(),
        })
    }

    /// URL for OCI distribution API endpoint
    pub fn registry_url(&self) -> Result<Url> {
        let hostname = if let Some(port) = self.port {
            format!("{}:{}", self.hostname, port)
        } else {
            self.hostname.clone()
        };
        let url = if self.hostname.starts_with("localhost") {
            format!("http://{}", hostname)
        } else {
            format!("https://{}", hostname)
        };
        Ok(Url::parse(&url)?)
    }
}
