use crate::error::{HyphaError, Result};
use oci_spec::image::{Arch, Descriptor, ImageIndex, ImageManifest, MediaType, Os, ToDockerV2S2};
use std::io::Read;
use ureq::{Agent, Request, Response};
use url::Url;

pub struct RegistryClient {
    agent: Agent,
    url: Url,
}

impl RegistryClient {
    pub fn new(url: Url) -> Result<RegistryClient> {
        Ok(RegistryClient {
            agent: Agent::new(),
            url,
        })
    }

    fn call(&mut self, req: Request) -> Result<Response> {
        Ok(req.call()?)
    }

    pub fn get_blob(&mut self, name: &str, descriptor: &Descriptor) -> Result<Vec<u8>> {
        let url = self
            .url
            .join(&format!("/v2/{}/blobs/{}", name, descriptor.digest()))?;
        let response = self.call(self.agent.get(url.as_str()))?;
        let mut buffer: Vec<u8> = Vec::new();
        response.into_reader().read_to_end(&mut buffer)?;
        Ok(buffer)
    }

    pub fn get_manifest(&mut self, name: &str, reference: &str) -> Result<ImageManifest> {
        let url = self
            .url
            .join(&format!("/v2/{}/manifests/{}", name, reference))?;
        let accept = format!(
            "{}, {}, {}",
            MediaType::ImageManifest.to_docker_v2s2()?,
            MediaType::ImageManifest,
            MediaType::ImageIndex,
        );
        let response = self.call(self.agent.get(url.as_str()).set("Accept", &accept))?;
        let content_type = response.header("Content-Type").ok_or_else(|| {
            HyphaError::new("registry response did not have a Content-Type header")
        })?;
        if content_type == MediaType::ImageIndex.to_string() {
            let index = ImageIndex::from_reader(response.into_reader())?;
            let descriptor = self
                .pick_manifest(index)
                .ok_or_else(|| HyphaError::new("unable to pick manifest from index"))?;
            return self.get_manifest(name, descriptor.digest());
        }
        let manifest = ImageManifest::from_reader(response.into_reader())?;
        Ok(manifest)
    }

    fn pick_manifest(&mut self, index: ImageIndex) -> Option<Descriptor> {
        for item in index.manifests() {
            if let Some(platform) = item.platform() {
                if *platform.os() == Os::Linux && *platform.architecture() == Arch::Amd64 {
                    return Some(item.clone());
                }
            }
        }
        None
    }
}
