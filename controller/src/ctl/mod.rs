pub mod cfgblk;

use crate::autoloop::AutoLoop;
use crate::image::cache::ImageCache;
use anyhow::{anyhow, Result};
use loopdev::LoopControl;
use std::fs;
use std::path::PathBuf;
use std::str::FromStr;
use uuid::Uuid;
use xenclient::XenClient;
use xenstore::client::XsdInterface;

pub mod console;
pub mod destroy;
pub mod launch;

pub struct ControllerContext {
    pub image_cache: ImageCache,
    pub autoloop: AutoLoop,
    pub xen: XenClient,
}

pub struct ContainerLoopInfo {
    pub device: String,
    pub file: String,
    pub delete: Option<String>,
}

pub struct ContainerInfo {
    pub uuid: Uuid,
    pub domid: u32,
    pub image: String,
    pub loops: Vec<ContainerLoopInfo>,
    pub ipv4: String,
    pub ipv6: String,
}

impl ControllerContext {
    pub fn new(store_path: String) -> Result<ControllerContext> {
        let mut image_cache_path = PathBuf::from(store_path);
        image_cache_path.push("cache");
        fs::create_dir_all(&image_cache_path)?;

        let xen = XenClient::open()?;
        image_cache_path.push("image");
        fs::create_dir_all(&image_cache_path)?;
        let image_cache = ImageCache::new(&image_cache_path)?;
        Ok(ControllerContext {
            image_cache,
            autoloop: AutoLoop::new(LoopControl::open()?),
            xen,
        })
    }

    pub fn list(&mut self) -> Result<Vec<ContainerInfo>> {
        let mut containers: Vec<ContainerInfo> = Vec::new();
        for domid_candidate in self.xen.store.list_any("/local/domain")? {
            let dom_path = format!("/local/domain/{}", domid_candidate);
            let uuid_string = match self
                .xen
                .store
                .read_string_optional(&format!("{}/hypha/uuid", &dom_path))?
            {
                None => continue,
                Some(value) => value,
            };
            let domid =
                u32::from_str(&domid_candidate).map_err(|_| anyhow!("failed to parse domid"))?;
            let uuid = Uuid::from_str(&uuid_string)?;
            let image = self
                .xen
                .store
                .read_string_optional(&format!("{}/hypha/image", &dom_path))?
                .unwrap_or("unknown".to_string());
            let loops = self
                .xen
                .store
                .read_string_optional(&format!("{}/hypha/loops", &dom_path))?
                .unwrap_or("".to_string());
            let ipv4 = self
                .xen
                .store
                .read_string_optional(&format!("{}/hypha/network/guest/ipv4", &dom_path))?
                .unwrap_or("unknown".to_string());
            let ipv6: String = self
                .xen
                .store
                .read_string_optional(&format!("{}/hypha/network/guest/ipv6", &dom_path))?
                .unwrap_or("unknown".to_string());
            let loops = ControllerContext::parse_loop_set(&loops);
            containers.push(ContainerInfo {
                uuid,
                domid,
                image,
                loops,
                ipv4,
                ipv6,
            });
        }
        Ok(containers)
    }

    pub fn resolve(&mut self, id: &str) -> Result<Option<ContainerInfo>> {
        for container in self.list()? {
            let uuid_string = container.uuid.to_string();
            let domid_string = container.domid.to_string();
            if uuid_string == id || domid_string == id || id == format!("hypha-{}", uuid_string) {
                return Ok(Some(container));
            }
        }
        Ok(None)
    }

    fn parse_loop_set(input: &str) -> Vec<ContainerLoopInfo> {
        let sets = input
            .split(',')
            .map(|x| x.to_string())
            .map(|x| x.split(':').map(|v| v.to_string()).collect::<Vec<String>>())
            .map(|x| (x[0].clone(), x[1].clone(), x[2].clone()))
            .collect::<Vec<(String, String, String)>>();
        sets.iter()
            .map(|(device, file, delete)| ContainerLoopInfo {
                device: device.clone(),
                file: file.clone(),
                delete: if delete == "none" {
                    None
                } else {
                    Some(delete.clone())
                },
            })
            .collect::<Vec<ContainerLoopInfo>>()
    }
}
