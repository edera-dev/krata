use std::collections::HashMap;

use anyhow::{anyhow, Result};
use bytes::Bytes;
use oci_spec::image::{Arch, Descriptor, ImageIndex, ImageManifest, MediaType, Os, ToDockerV2S2};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
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
    token: Option<String>,
}

impl OciRegistryClient {
    pub fn new(url: Url, platform: OciRegistryPlatform) -> Result<OciRegistryClient> {
        Ok(OciRegistryClient {
            agent: Client::new(),
            url,
            platform,
            token: None,
        })
    }

    async fn call(&mut self, mut req: RequestBuilder) -> Result<Response> {
        if let Some(ref token) = self.token {
            req = req.bearer_auth(token);
        }
        let req_first_try = req.try_clone().ok_or(anyhow!("request is not clonable"))?;
        let response = self.agent.execute(req_first_try.build()?).await?;
        if response.status() == StatusCode::UNAUTHORIZED && self.token.is_none() {
            let Some(www_authenticate) = response.headers().get("www-authenticate") else {
                return Err(anyhow!("not authorized to perform this action"));
            };

            let www_authenticate = www_authenticate.to_str()?;
            if !www_authenticate.starts_with("Bearer ") {
                return Err(anyhow!("unknown authentication scheme"));
            }

            let details = &www_authenticate[7..];
            let details = details
                .split(',')
                .map(|x| x.split('='))
                .map(|mut x| (x.next(), x.next()))
                .filter(|(key, value)| key.is_some() && value.is_some())
                .map(|(key, value)| {
                    (
                        key.unwrap().trim().to_lowercase(),
                        value.unwrap().trim().to_string(),
                    )
                })
                .map(|(key, value)| (key, value.trim_matches('\"').to_string()))
                .collect::<HashMap<_, _>>();
            let realm = details.get("realm");
            let service = details.get("service");
            let scope = details.get("scope");
            if realm.is_none() || service.is_none() || scope.is_none() {
                return Err(anyhow!(
                    "unknown authentication scheme: realm, service, and scope are required"
                ));
            }
            let mut url = Url::parse(realm.unwrap())?;
            url.query_pairs_mut()
                .append_pair("service", service.unwrap())
                .append_pair("scope", scope.unwrap());
            let token_response = self.agent.get(url.clone()).send().await?;
            if token_response.status() != StatusCode::OK {
                return Err(anyhow!(
                    "failed to acquire token via {}: status {}",
                    url,
                    token_response.status()
                ));
            }
            let token_bytes = token_response.bytes().await?;
            let token = serde_json::from_slice::<serde_json::Value>(&token_bytes)?;
            let token = token
                .get("token")
                .and_then(|x| x.as_str())
                .ok_or(anyhow!("token key missing from response"))?;
            self.token = Some(token.to_string());
            return Ok(self.agent.execute(req.bearer_auth(token).build()?).await?);
        }

        if !response.status().is_success() {
            return Err(anyhow!(
                "request to {} failed: status {}",
                req.build()?.url(),
                response.status()
            ));
        }

        Ok(response)
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
