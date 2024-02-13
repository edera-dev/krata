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

    pub fn perform(&mut self, id: &str) -> Result<Uuid> {
        let info = self
            .context
            .resolve(id)?
            .ok_or_else(|| anyhow!("unable to resolve container: {}", id))?;
        let domid = info.domid;
        let mut store = XsdClient::open()?;
        let dom_path = store.get_domain_path(domid)?;
        let uuid = match store.read_string_optional(format!("{}/hypha/uuid", dom_path).as_str())? {
            None => {
                return Err(anyhow!(
                    "domain {} was not found or not created by hypha",
                    domid
                ))
            }
            Some(value) => value,
        };
        if uuid.is_empty() {
            return Err(anyhow!("unable to find hypha uuid based on the domain",));
        }
        let uuid = Uuid::parse_str(&uuid)?;
        let loops = store.read_string(format!("{}/hypha/loops", dom_path).as_str())?;
        let loops = ControllerContext::parse_loop_set(&loops);
        self.context.xen.destroy(domid)?;
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
