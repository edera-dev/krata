use anyhow::Result;
use std::fmt;
use url::Url;

const DOCKER_HUB_MIRROR: &str = "mirror.gcr.io";
const DEFAULT_IMAGE_TAG: &str = "latest";

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageName {
    pub hostname: String,
    pub port: Option<u16>,
    pub name: String,
    pub reference: String,
}

impl fmt::Display for ImageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if DOCKER_HUB_MIRROR == self.hostname && self.port.is_none() {
            if self.name.starts_with("library/") {
                write!(f, "{}:{}", &self.name[8..], self.reference)
            } else {
                write!(f, "{}:{}", self.name, self.reference)
            }
        } else if let Some(port) = self.port {
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
        let full_name = name.to_string();
        let name = full_name.clone();
        let (mut hostname, mut name) = name
            .split_once('/')
            .map(|x| (x.0.to_string(), x.1.to_string()))
            .unwrap_or_else(|| (DOCKER_HUB_MIRROR.to_string(), format!("library/{}", name)));

        // heuristic to find any docker hub image formats
        // that may be in the hostname format. for example:
        // abc/xyz:latest will trigger this if check, but abc.io/xyz:latest will not,
        // and neither will abc/hello/xyz:latest
        if !hostname.contains('.') && full_name.chars().filter(|x| *x == '/').count() == 1 {
            name = format!("{}/{}", hostname, name);
            hostname = DOCKER_HUB_MIRROR.to_string();
        }

        let (hostname, port) = if let Some((hostname, port)) = hostname
            .split_once(':')
            .map(|x| (x.0.to_string(), x.1.to_string()))
        {
            (hostname, Some(str::parse(&port)?))
        } else {
            (hostname, None)
        };
        let (name, reference) = name
            .split_once(':')
            .map(|x| (x.0.to_string(), x.1.to_string()))
            .unwrap_or((name.to_string(), DEFAULT_IMAGE_TAG.to_string()));
        Ok(ImageName {
            hostname,
            port,
            name,
            reference,
        })
    }

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
