use std::{fs, path::PathBuf};

use anyhow::{anyhow, Result};
use uuid::Uuid;
use xenstore::client::{XsdClient, XsdInterface};

use super::ControllerContext;

pub struct ControllerDestroy<'a> {
    context: &'a mut ControllerContext,
}

impl ControllerDestroy<'_> {
    pub fn new(context: &mut ControllerContext) -> ControllerDestroy<'_> {
        ControllerDestroy { context }
    }

    pub async fn perform(&mut self, id: &str) -> Result<Uuid> {
        let info = self
            .context
            .resolve(id)
            .await?
            .ok_or_else(|| anyhow!("unable to resolve container: {}", id))?;
        let domid = info.domid;
        let mut store = XsdClient::open().await?;
        let dom_path = store.get_domain_path(domid).await?;
        let uuid = match store
            .read_string(format!("{}/krata/uuid", dom_path).as_str())
            .await?
        {
            None => {
                return Err(anyhow!(
                    "domain {} was not found or not created by krata",
                    domid
                ))
            }
            Some(value) => value,
        };
        if uuid.is_empty() {
            return Err(anyhow!("unable to find krata uuid based on the domain",));
        }
        let uuid = Uuid::parse_str(&uuid)?;
        let loops = store
            .read_string(format!("{}/krata/loops", dom_path).as_str())
            .await?;
        let loops = ControllerContext::parse_loop_set(&loops);
        self.context.xen.destroy(domid).await?;
        for info in &loops {
            self.context.autoloop.unloop(&info.device)?;
            match &info.delete {
                None => {}
                Some(delete) => {
                    let delete_path = PathBuf::from(delete);
                    if delete_path.is_file() || delete_path.is_symlink() {
                        fs::remove_file(&delete_path)?;
                    } else if delete_path.is_dir() {
                        fs::remove_dir_all(&delete_path)?;
                    }
                }
            }
        }
        Ok(uuid)
    }
}
