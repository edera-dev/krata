use anyhow::Result;
use std::fmt;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ImageName {
    pub hostname: String,
    pub port: Option<u16>,
    pub name: String,
    pub reference: Option<String>,
    pub digest: Option<String>,
}

impl fmt::Display for ImageName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut suffix = String::new();

        if let Some(ref reference) = self.reference {
            suffix.push(':');
            suffix.push_str(reference);
        }

        if let Some(ref digest) = self.digest {
            suffix.push('@');
            suffix.push_str(digest);
        }

        if ImageName::DOCKER_HUB_MIRROR == self.hostname && self.port.is_none() {
            if self.name.starts_with("library/") {
                write!(f, "{}{}", &self.name[8..], suffix)
            } else {
                write!(f, "{}{}", self.name, suffix)
            }
        } else if let Some(port) = self.port {
            write!(f, "{}:{}/{}{}", self.hostname, port, self.name, suffix)
        } else {
            write!(f, "{}/{}{}", self.hostname, self.name, suffix)
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
    pub const DOCKER_HUB_MIRROR: &'static str = "registry.docker.io";
    pub const DEFAULT_IMAGE_TAG: &'static str = "latest";

    pub fn parse(name: &str) -> Result<Self> {
        let full_name = name.to_string();
        let name = full_name.clone();
        let (mut hostname, mut name) = name
            .split_once('/')
            .map(|x| (x.0.to_string(), x.1.to_string()))
            .unwrap_or_else(|| {
                (
                    ImageName::DOCKER_HUB_MIRROR.to_string(),
                    format!("library/{}", name),
                )
            });

        // heuristic to find any docker hub image formats
        // that may be in the hostname format. for example:
        // abc/xyz:latest will trigger this if check, but abc.io/xyz:latest will not,
        // and neither will abc/hello/xyz:latest
        if !hostname.contains('.') && full_name.chars().filter(|x| *x == '/').count() == 1 {
            name = format!("{}/{}", hostname, name);
            hostname = ImageName::DOCKER_HUB_MIRROR.to_string();
        }

        let (hostname, port) = if let Some((hostname, port)) = hostname
            .split_once(':')
            .map(|x| (x.0.to_string(), x.1.to_string()))
        {
            (hostname, Some(str::parse(&port)?))
        } else {
            (hostname, None)
        };

        let name_has_digest = if name.contains('@') {
            let digest_start = name.chars().position(|c| c == '@');
            let ref_start = name.chars().position(|c| c == ':');
            if let (Some(digest_start), Some(ref_start)) = (digest_start, ref_start) {
                digest_start < ref_start
            } else {
                true
            }
        } else {
            false
        };

        let (name, digest) = if name_has_digest {
            name.split_once('@')
                .map(|(name, digest)| (name.to_string(), Some(digest.to_string())))
                .unwrap_or_else(|| (name, None))
        } else {
            (name, None)
        };

        let (name, reference) = if name.contains(':') {
            name.split_once(':')
                .map(|(name, reference)| (name.to_string(), Some(reference.to_string())))
                .unwrap_or((name, None))
        } else {
            (name, None)
        };

        let (reference, digest) = if let Some(reference) = reference {
            if let Some(digest) = digest {
                (Some(reference), Some(digest))
            } else {
                reference
                    .split_once('@')
                    .map(|(reff, digest)| (Some(reff.to_string()), Some(digest.to_string())))
                    .unwrap_or_else(|| (Some(reference), None))
            }
        } else {
            (None, digest)
        };

        Ok(ImageName {
            hostname,
            port,
            name,
            reference,
            digest,
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
