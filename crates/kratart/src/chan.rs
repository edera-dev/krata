use std::collections::HashMap;

use anyhow::Result;
use xenevtchn::EventChannel;
use xengnt::{sys::GrantRef, GrantTab};
use xenstore::{XsdClient, XsdInterface};

#[repr(C)]
struct XenConsoleInterface {
    input: [u8; 1024],
    output: [u8; 2048],
    in_cons: u32,
    in_prod: u32,
    out_cons: u32,
    out_prod: u32,
}

pub struct KrataChannelService {
    backends: HashMap<(u32, u32), KrataChannelBackend>,
    evtchn: EventChannel,
    store: XsdClient,
}

impl KrataChannelService {
    pub fn new(evtchn: EventChannel, store: XsdClient) -> Result<KrataChannelService> {
        Ok(KrataChannelService {
            backends: HashMap::new(),
            evtchn,
            store,
        })
    }

    pub async fn init(&mut self) -> Result<()> {
        let domains = self.store.list("/local/domain/0/backend/console").await?;
        for domid_string in domains {
            let domid = domid_string.parse::<u32>()?;
            let domid_path = format!("/local/domain/0/backend/console/{}", domid);
            for id_string in self.store.list(&domid_path).await? {
                let id = id_string.parse::<u32>()?;
                let console_path = format!(
                    "/local/domain/0/backend/console/{}/{}",
                    domid_string, id_string
                );
                let Some(frontend_path) = self
                    .store
                    .read_string(format!("{}/frontend", console_path))
                    .await?
                else {
                    continue;
                };
                let Some(typ) = self
                    .store
                    .read_string(format!("{}/type", frontend_path))
                    .await?
                else {
                    continue;
                };

                if typ != "krata-channel" {
                    continue;
                }

                let Some(ring_ref_string) = self
                    .store
                    .read_string(format!("{}/ring-ref", frontend_path))
                    .await?
                else {
                    continue;
                };

                let Some(port_string) = self
                    .store
                    .read_string(format!("{}/port", frontend_path))
                    .await?
                else {
                    continue;
                };

                let ring_ref = ring_ref_string.parse::<u64>()?;
                let port = port_string.parse::<u32>()?;
                let backend = KrataChannelBackend {
                    backend: console_path.clone(),
                    domid,
                    ring_ref,
                    port,
                    store: self.store.clone(),
                    evtchn: self.evtchn.clone(),
                    grant: GrantTab::open()?,
                };

                backend.init().await?;
                self.backends.insert((domid, id), backend);
            }
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct KrataChannelBackend {
    backend: String,
    domid: u32,
    ring_ref: u64,
    port: u32,
    store: XsdClient,
    evtchn: EventChannel,
    grant: GrantTab,
}

impl KrataChannelBackend {
    pub async fn init(&self) -> Result<()> {
        self.store.write_string(&self.backend, "4").await?;
        Ok(())
    }

    pub async fn read(&self) -> Result<()> {
        let memory = self.grant.map_grant_refs(
            vec![GrantRef {
                domid: self.domid,
                reference: self.ring_ref as u32,
            }],
            true,
            true,
        )?;
        let interface = memory.ptr() as *mut XenConsoleInterface;
        let mut channel = self.evtchn.bind(self.domid, self.port).await?;
        unsafe { self.read_buffer(channel.local_port, interface).await? };
        loop {
            channel.receiver.recv().await?;
            unsafe { self.read_buffer(channel.local_port, interface).await? };
            channel.unmask_sender.send(channel.local_port).await?;
        }
    }

    async unsafe fn read_buffer(
        &self,
        local_port: u32,
        interface: *mut XenConsoleInterface,
    ) -> Result<()> {
        let mut cons = (*interface).out_cons;
        let prod = (*interface).out_prod;
        let size = prod - cons;
        if size == 0 || size > 2048 {
            return Ok(());
        }
        let mut data: Vec<u8> = Vec::new();
        loop {
            if cons == prod {
                break;
            }
            data.push((*interface).output[cons as usize]);
            cons += 1;
        }
        (*interface).out_cons = cons;
        self.evtchn.notify(local_port).await?;
        Ok(())
    }
}
