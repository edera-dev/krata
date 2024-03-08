use anyhow::{anyhow, Result};
use bytes::Bytes;
use oci_spec::image::{Arch, Descriptor, ImageIndex, ImageManifest, MediaType, Os, ToDockerV2S2};
use reqwest::{Client, RequestBuilder, Response};
use tokio::{fs::File, io::AsyncWriteExt};
use url::Url;

#[derive(Clone, Debug)]
pub struct OciRegistryPlatform {
    pub os: Os,
    pub arch: Arch,
}

impl OciRegistryPlatform {
    #[cfg(target_arch = "x86_64")]
    const CURRENT_ARCH: Arch = Arch::Amd64;
    #[cfg(target_arch = "aarch64")]
    const CURRENT_ARCH: Arch = Arch::ARM64;

    pub fn new(os: Os, arch: Arch) -> OciRegistryPlatform {
        OciRegistryPlatform { os, arch }
    }

    pub fn current() -> OciRegistryPlatform {
        OciRegistryPlatform {
            os: Os::Linux,
            arch: OciRegistryPlatform::CURRENT_ARCH,
        }
    }
}

pub struct OciRegistryClient {
    agent: Client,
    url: Url,
    platform: OciRegistryPlatform,
}

impl OciRegistryClient {
    pub fn new(url: Url, platform: OciRegistryPlatform) -> Result<OciRegistryClient> {
        Ok(OciRegistryClient {
            agent: Client::new(),
            url,
            platform,
        })
    }

    async fn call(&mut self, req: RequestBuilder) -> Result<Response> {
        self.agent.execute(req.build()?).await.map_err(|x| x.into())
    }

    pub async fn get_blob<N: AsRef<str>>(
        &mut self,
        name: N,
        descriptor: &Descriptor,
    ) -> Result<Bytes> {
        let url = self.url.join(&format!(
            "/v2/{}/blobs/{}",
            name.as_ref(),
            descriptor.digest()
        ))?;
        let response = self.call(self.agent.get(url.as_str())).await?;
        Ok(response.bytes().await?)
    }

    pub async fn write_blob_to_file<N: AsRef<str>>(
        &mut self,
        name: N,
        descriptor: &Descriptor,
        mut dest: File,
    ) -> Result<u64> {
        let url = self.url.join(&format!(
            "/v2/{}/blobs/{}",
            name.as_ref(),
            descriptor.digest()
        ))?;
        let mut response = self.call(self.agent.get(url.as_str())).await?;
        let mut size: u64 = 0;
        while let Some(chunk) = response.chunk().await? {
            dest.write_all(&chunk).await?;
            size += chunk.len() as u64;
        }
        Ok(size)
    }

    async fn get_raw_manifest_with_digest<N: AsRef<str>, R: AsRef<str>>(
        &mut self,
        name: N,
        reference: R,
    ) -> Result<(ImageManifest, String)> {
        let url = self.url.join(&format!(
            "/v2/{}/manifests/{}",
            name.as_ref(),
            reference.as_ref()
        ))?;
        let accept = format!(
            "{}, {}, {}, {}",
            MediaType::ImageManifest.to_docker_v2s2()?,
            MediaType::ImageManifest,
            MediaType::ImageIndex,
            MediaType::ImageIndex.to_docker_v2s2()?,
        );
        let response = self
            .call(self.agent.get(url.as_str()).header("Accept", &accept))
            .await?;
        let digest = response
            .headers()
            .get("Docker-Content-Digest")
            .ok_or_else(|| anyhow!("fetching manifest did not yield a content digest"))?
            .to_str()?
            .to_string();
        let manifest = serde_json::from_str(&response.text().await?)?;
        Ok((manifest, digest))
    }

    pub async fn get_manifest_with_digest<N: AsRef<str>, R: AsRef<str>>(
        &mut self,
        name: N,
        reference: R,
    ) -> Result<(ImageManifest, String)> {
        let url = self.url.join(&format!(
            "/v2/{}/manifests/{}",
            name.as_ref(),
            reference.as_ref()
        ))?;
        let accept = format!(
            "{}, {}, {}, {}",
            MediaType::ImageManifest.to_docker_v2s2()?,
            MediaType::ImageManifest,
            MediaType::ImageIndex,
            MediaType::ImageIndex.to_docker_v2s2()?,
        );
        let response = self
            .call(self.agent.get(url.as_str()).header("Accept", &accept))
            .await?;
        let content_type = response
            .headers()
            .get("Content-Type")
            .ok_or_else(|| anyhow!("registry response did not have a Content-Type header"))?
            .to_str()?;
        if content_type == MediaType::ImageIndex.to_string()
            || content_type == MediaType::ImageIndex.to_docker_v2s2()?
        {
            let index = serde_json::from_str(&response.text().await?)?;
            let descriptor = self
                .pick_manifest(index)
                .ok_or_else(|| anyhow!("unable to pick manifest from index"))?;
            return self
                .get_raw_manifest_with_digest(name, descriptor.digest())
                .await;
        }
        let digest = response
            .headers()
            .get("Docker-Content-Digest")
            .ok_or_else(|| anyhow!("fetching manifest did not yield a content digest"))?
            .to_str()?
            .to_string();
        let manifest = serde_json::from_str(&response.text().await?)?;
        Ok((manifest, digest))
    }

    fn pick_manifest(&mut self, index: ImageIndex) -> Option<Descriptor> {
        for item in index.manifests() {
            if let Some(platform) = item.platform() {
                if *platform.os() == self.platform.os
                    && *platform.architecture() == self.platform.arch
                {
                    return Some(item.clone());
                }
            }
        }
        None
    }
}
