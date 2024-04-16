use indexmap::IndexMap;
use std::sync::Arc;
use tokio::{
    sync::{broadcast, Mutex},
    task::JoinHandle,
};

const OCI_PROGRESS_QUEUE_LEN: usize = 100;

#[derive(Clone, Debug)]
pub struct OciProgress {
    pub phase: OciProgressPhase,
    pub layers: IndexMap<String, OciProgressLayer>,
    pub value: u64,
    pub total: u64,
}

impl Default for OciProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl OciProgress {
    pub fn new() -> Self {
        OciProgress {
            phase: OciProgressPhase::Resolving,
            layers: IndexMap::new(),
            value: 0,
            total: 1,
        }
    }

    pub fn add_layer(&mut self, id: &str, size: usize) {
        self.layers.insert(
            id.to_string(),
            OciProgressLayer {
                id: id.to_string(),
                phase: OciProgressLayerPhase::Waiting,
                value: 0,
                total: size as u64,
            },
        );
    }

    pub fn downloading_layer(&mut self, id: &str, downloaded: usize, total: usize) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Downloading;
            entry.value = downloaded as u64;
            entry.total = total as u64;
        }
    }

    pub fn downloaded_layer(&mut self, id: &str) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Downloaded;
            entry.value = entry.total;
        }
    }

    pub fn extracting_layer(&mut self, id: &str, extracted: usize, total: usize) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Extracting;
            entry.value = extracted as u64;
            entry.total = total as u64;
        }
    }

    pub fn extracted_layer(&mut self, id: &str) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Extracted;
            entry.value = entry.total;
        }
    }
}

#[derive(Clone, Debug)]
pub enum OciProgressPhase {
    Resolving,
    Resolved,
    ConfigAcquire,
    LayerAcquire,
    Packing,
    Complete,
}

#[derive(Clone, Debug)]
pub struct OciProgressLayer {
    pub id: String,
    pub phase: OciProgressLayerPhase,
    pub value: u64,
    pub total: u64,
}

#[derive(Clone, Debug)]
pub enum OciProgressLayerPhase {
    Waiting,
    Downloading,
    Downloaded,
    Extracting,
    Extracted,
}

#[derive(Clone)]
pub struct OciProgressContext {
    sender: broadcast::Sender<OciProgress>,
}

impl OciProgressContext {
    pub fn create() -> (OciProgressContext, broadcast::Receiver<OciProgress>) {
        let (sender, receiver) = broadcast::channel(OCI_PROGRESS_QUEUE_LEN);
        (OciProgressContext::new(sender), receiver)
    }

    pub fn new(sender: broadcast::Sender<OciProgress>) -> OciProgressContext {
        OciProgressContext { sender }
    }

    pub fn update(&self, progress: &OciProgress) {
        let _ = self.sender.send(progress.clone());
    }

    pub fn subscribe(&self) -> broadcast::Receiver<OciProgress> {
        self.sender.subscribe()
    }
}

#[derive(Clone)]
pub struct OciBoundProgress {
    context: OciProgressContext,
    instance: Arc<Mutex<OciProgress>>,
}

impl OciBoundProgress {
    pub fn new(context: OciProgressContext, progress: OciProgress) -> OciBoundProgress {
        OciBoundProgress {
            context,
            instance: Arc::new(Mutex::new(progress)),
        }
    }

    pub async fn update(&self, function: impl FnOnce(&mut OciProgress)) {
        let mut progress = self.instance.lock().await;
        function(&mut progress);
        self.context.update(&progress);
    }

    pub fn update_blocking(&self, function: impl FnOnce(&mut OciProgress)) {
        let mut progress = self.instance.blocking_lock();
        function(&mut progress);
        self.context.update(&progress);
    }

    pub async fn also_update(&self, context: OciProgressContext) -> JoinHandle<()> {
        let progress = self.instance.lock().await.clone();
        context.update(&progress);
        let mut receiver = self.context.subscribe();
        tokio::task::spawn(async move {
            while let Ok(progress) = receiver.recv().await {
                match context.sender.send(progress) {
                    Ok(_) => {}
                    Err(_) => {
                        break;
                    }
                }
            }
        })
    }
}
