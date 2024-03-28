use std::{
    collections::HashMap,
    sync::atomic::{fence, Ordering},
    time::Duration,
};

use anyhow::{anyhow, Result};
use log::{error, info};
use tokio::{
    select,
    sync::{
        broadcast,
        mpsc::{channel, Receiver, Sender},
    },
    task::JoinHandle,
    time::sleep,
};
use xenevtchn::EventChannel;
use xengnt::{sys::GrantRef, GrantTab, MappedMemory};
use xenstore::{XsdClient, XsdInterface};

const KRATA_SINGLE_CHANNEL_QUEUE_LEN: usize = 100;

#[repr(C)]
struct XenConsoleInterface {
    input: [u8; XenConsoleInterface::INPUT_SIZE],
    output: [u8; XenConsoleInterface::OUTPUT_SIZE],
    in_cons: u32,
    in_prod: u32,
    out_cons: u32,
    out_prod: u32,
}

unsafe impl Send for XenConsoleInterface {}

impl XenConsoleInterface {
    const INPUT_SIZE: usize = 1024;
    const OUTPUT_SIZE: usize = 2048;
}

pub struct KrataChannelService {
    backends: HashMap<(u32, u32), KrataChannelBackend>,
    evtchn: EventChannel,
    store: XsdClient,
    gnttab: GrantTab,
}

impl KrataChannelService {
    pub fn new(
        evtchn: EventChannel,
        store: XsdClient,
        gnttab: GrantTab,
    ) -> Result<KrataChannelService> {
        Ok(KrataChannelService {
            backends: HashMap::new(),
            evtchn,
            store,
            gnttab,
        })
    }

    pub async fn watch(&mut self) -> Result<()> {
        self.scan_all_backends().await?;
        let mut watch_handle = self.store.create_watch().await?;
        self.store
            .bind_watch(&watch_handle, "/local/domain/0/backend/console".to_string())
            .await?;
        loop {
            let Some(_) = watch_handle.receiver.recv().await else {
                break;
            };

            self.scan_all_backends().await?;
        }
        Ok(())
    }

    async fn ensure_backend_exists(&mut self, domid: u32, id: u32, path: String) -> Result<()> {
        if self.backends.contains_key(&(domid, id)) {
            return Ok(());
        }
        let Some(frontend_path) = self.store.read_string(format!("{}/frontend", path)).await?
        else {
            return Ok(());
        };
        let Some(typ) = self
            .store
            .read_string(format!("{}/type", frontend_path))
            .await?
        else {
            return Ok(());
        };

        if typ != "krata-channel" {
            return Ok(());
        }

        let backend = KrataChannelBackend::new(
            path.clone(),
            frontend_path.clone(),
            domid,
            id,
            self.store.clone(),
            self.evtchn.clone(),
            self.gnttab.clone(),
        )
        .await?;
        self.backends.insert((domid, id), backend);
        Ok(())
    }

    async fn scan_all_backends(&mut self) -> Result<()> {
        let domains = self.store.list("/local/domain/0/backend/console").await?;
        let mut seen: Vec<(u32, u32)> = Vec::new();
        for domid_string in &domains {
            let domid = domid_string.parse::<u32>()?;
            let domid_path = format!("/local/domain/0/backend/console/{}", domid);
            for id_string in self.store.list(&domid_path).await? {
                let id = id_string.parse::<u32>()?;
                let console_path = format!(
                    "/local/domain/0/backend/console/{}/{}",
                    domid_string, id_string
                );
                self.ensure_backend_exists(domid, id, console_path).await?;
                seen.push((domid, id));
            }
        }

        let mut gone: Vec<(u32, u32)> = Vec::new();
        for backend in self.backends.keys() {
            if !seen.contains(backend) {
                gone.push(*backend);
            }
        }

        for item in gone {
            if let Some(backend) = self.backends.remove(&item) {
                drop(backend);
            }
        }

        Ok(())
    }
}

pub struct KrataChannelBackend {
    pub domid: u32,
    pub id: u32,
    pub receiver: Receiver<Vec<u8>>,
    pub sender: Sender<Vec<u8>>,
    task: JoinHandle<()>,
}

impl Drop for KrataChannelBackend {
    fn drop(&mut self) {
        self.task.abort();
        info!(
            "destroyed channel backend for domain {} channel {}",
            self.domid, self.id
        );
    }
}

impl KrataChannelBackend {
    pub async fn new(
        backend: String,
        frontend: String,
        domid: u32,
        id: u32,
        store: XsdClient,
        evtchn: EventChannel,
        gnttab: GrantTab,
    ) -> Result<KrataChannelBackend> {
        let processor = KrataChannelBackendProcessor {
            backend,
            frontend,
            domid,
            id,
            store,
            evtchn,
            gnttab,
        };

        let (output_sender, output_receiver) = channel(KRATA_SINGLE_CHANNEL_QUEUE_LEN);
        let (input_sender, input_receiver) = channel(KRATA_SINGLE_CHANNEL_QUEUE_LEN);

        let task = processor.launch(output_sender, input_receiver).await?;
        Ok(KrataChannelBackend {
            domid,
            id,
            task,
            receiver: output_receiver,
            sender: input_sender,
        })
    }
}

#[derive(Clone)]
pub struct KrataChannelBackendProcessor {
    backend: String,
    frontend: String,
    id: u32,
    domid: u32,
    store: XsdClient,
    evtchn: EventChannel,
    gnttab: GrantTab,
}

impl KrataChannelBackendProcessor {
    async fn init(&self) -> Result<()> {
        self.store
            .write_string(format!("{}/state", self.backend), "3")
            .await?;
        info!(
            "created channel backend for domain {} channel {}",
            self.domid, self.id
        );
        Ok(())
    }

    async fn on_frontend_state_change(&self) -> Result<bool> {
        let state = self
            .store
            .read_string(format!("{}/state", self.backend))
            .await?
            .unwrap_or("0".to_string())
            .parse::<u32>()?;
        if state == 3 {
            return Ok(true);
        }
        Ok(false)
    }

    async fn on_self_state_change(&self) -> Result<bool> {
        let state = self
            .store
            .read_string(format!("{}/state", self.backend))
            .await?
            .unwrap_or("0".to_string())
            .parse::<u32>()?;
        if state == 5 {
            return Ok(true);
        }
        Ok(false)
    }

    async fn launch(
        &self,
        output_sender: Sender<Vec<u8>>,
        input_receiver: Receiver<Vec<u8>>,
    ) -> Result<JoinHandle<()>> {
        let owned = self.clone();
        Ok(tokio::task::spawn(async move {
            if let Err(error) = owned.processor(output_sender, input_receiver).await {
                error!("failed to process krata channel: {}", error);
            }
            let _ = owned
                .store
                .write_string(format!("{}/state", owned.backend), "6")
                .await;
        }))
    }

    async fn processor(
        &self,
        sender: Sender<Vec<u8>>,
        mut receiver: Receiver<Vec<u8>>,
    ) -> Result<()> {
        self.init().await?;
        let mut frontend_state_change = self.store.create_watch().await?;
        self.store
            .bind_watch(&frontend_state_change, format!("{}/state", self.frontend))
            .await?;

        let (ring_ref, port) = loop {
            match frontend_state_change.receiver.recv().await {
                Some(_) => {
                    if self.on_frontend_state_change().await? {
                        let mut tries = 0;
                        let (ring_ref, port) = loop {
                            let ring_ref = self
                                .store
                                .read_string(format!("{}/ring-ref", self.frontend))
                                .await?;
                            let port = self
                                .store
                                .read_string(format!("{}/port", self.frontend))
                                .await?;

                            if (ring_ref.is_none() || port.is_none()) && tries < 10 {
                                tries += 1;
                                self.store
                                    .write_string(format!("{}/state", self.backend), "4")
                                    .await?;
                                sleep(Duration::from_millis(250)).await;
                                continue;
                            }
                            break (ring_ref, port);
                        };

                        if ring_ref.is_none() || port.is_none() {
                            return Err(anyhow!("frontend did not give ring-ref and port"));
                        }

                        let Ok(ring_ref) = ring_ref.unwrap().parse::<u64>() else {
                            return Err(anyhow!("frontend gave invalid ring-ref"));
                        };

                        let Ok(port) = port.unwrap().parse::<u32>() else {
                            return Err(anyhow!("frontend gave invalid port"));
                        };

                        break (ring_ref, port);
                    }
                }

                None => {
                    return Ok(());
                }
            }
        };

        self.store
            .write_string(format!("{}/state", self.backend), "4")
            .await?;
        let memory = self.gnttab.map_grant_refs(
            vec![GrantRef {
                domid: self.domid,
                reference: ring_ref as u32,
            }],
            true,
            true,
        )?;
        let mut channel = self.evtchn.bind(self.domid, port).await?;
        unsafe {
            let buffer = self.read_output_buffer(channel.local_port, &memory).await?;
            if !buffer.is_empty() {
                sender.send(buffer).await?;
            }
        };

        let mut self_state_change = self.store.create_watch().await?;
        self.store
            .bind_watch(&self_state_change, format!("{}/state", self.backend))
            .await?;
        loop {
            select! {
                x = self_state_change.receiver.recv() => match x {
                    Some(_) => {
                        match self.on_self_state_change().await {
                            Err(error) => {
                                error!("failed to process state change for domain {} channel {}: {}", self.domid, self.id, error);
                            },

                            Ok(stop) => {
                                if stop {
                                    break;
                                }
                            }
                        }
                    },

                    None => {
                        break;
                    }
                },

                x = receiver.recv() => match x {
                    Some(data) => {
                        let mut index = 0;
                        loop {
                            if index >= data.len() {
                                break;
                            }
                            let interface = memory.ptr() as *mut XenConsoleInterface;
                            let cons = unsafe { (*interface).in_cons };
                            let mut prod = unsafe { (*interface).in_prod };
                            fence(Ordering::Release);
                            let space = (prod - cons) as usize;
                            if space > XenConsoleInterface::INPUT_SIZE {
                                error!("channel for domid {} has an invalid input space of {}", self.domid, space);
                            }
                            let free = XenConsoleInterface::INPUT_SIZE - space;
                            let want = data.len().min(free);
                            let buffer = &data[index..want];
                            for b in buffer {
                                unsafe { (*interface).input[prod as usize & (XenConsoleInterface::INPUT_SIZE - 1)] = *b; };
                                prod += 1;
                            }
                            fence(Ordering::Release);
                            unsafe { (*interface).in_prod = prod; };
                            self.evtchn.notify(channel.local_port).await?;
                            index += want;
                        }
                    },

                    None => {
                        break;
                    }
                },

                x = channel.receiver.recv() => match x {
                    Ok(_) => {
                        unsafe {
                            let buffer = self.read_output_buffer(channel.local_port, &memory).await?;
                            if !buffer.is_empty() {
                                sender.send(buffer).await?;
                            }
                        };
                        channel.unmask_sender.send(channel.local_port).await?;
                    },

                    Err(error) => {
                        match error {
                            broadcast::error::RecvError::Closed => {
                                break;
                            },
                            error => {
                                return Err(anyhow!("failed to receive event notification: {}", error));
                            }
                        }
                    }
                }
            };
        }
        Ok(())
    }

    async unsafe fn read_output_buffer<'a>(
        &self,
        local_port: u32,
        memory: &MappedMemory<'a>,
    ) -> Result<Vec<u8>> {
        let interface = memory.ptr() as *mut XenConsoleInterface;
        let mut cons = (*interface).out_cons;
        let prod = (*interface).out_prod;
        fence(Ordering::Release);
        let size = prod - cons;
        let mut data: Vec<u8> = Vec::new();
        if size == 0 || size as usize > XenConsoleInterface::OUTPUT_SIZE {
            return Ok(data);
        }
        loop {
            if cons == prod {
                break;
            }
            data.push((*interface).output[cons as usize & (XenConsoleInterface::OUTPUT_SIZE - 1)]);
            cons += 1;
        }
        fence(Ordering::AcqRel);
        (*interface).out_cons = cons;
        self.evtchn.notify(local_port).await?;
        Ok(data)
    }
}
