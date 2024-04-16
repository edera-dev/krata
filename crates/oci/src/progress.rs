use indexmap::IndexMap;
use std::sync::Arc;
use tokio::{
    sync::{watch, Mutex},
    task::JoinHandle,
};

#[derive(Clone, Debug)]
pub struct OciProgress {
    pub phase: OciProgressPhase,
    pub digest: Option<String>,
    pub layers: IndexMap<String, OciProgressLayer>,
    pub indication: OciProgressIndication,
}

impl Default for OciProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl OciProgress {
    pub fn new() -> Self {
        OciProgress {
            phase: OciProgressPhase::Started,
            digest: None,
            layers: IndexMap::new(),
            indication: OciProgressIndication::Hidden,
        }
    }

    pub fn start_resolving(&mut self) {
        self.phase = OciProgressPhase::Resolving;
        self.indication = OciProgressIndication::Spinner { message: None };
    }

    pub fn resolved(&mut self, digest: &str) {
        self.digest = Some(digest.to_string());
        self.indication = OciProgressIndication::Hidden;
    }

    pub fn add_layer(&mut self, id: &str) {
        self.layers.insert(
            id.to_string(),
            OciProgressLayer {
                id: id.to_string(),
                phase: OciProgressLayerPhase::Waiting,
                indication: OciProgressIndication::Spinner { message: None },
            },
        );
    }

    pub fn downloading_layer(&mut self, id: &str, downloaded: u64, total: u64) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Downloading;
            entry.indication = OciProgressIndication::ProgressBar {
                message: None,
                current: downloaded,
                total,
                bytes: true,
            };
        }
    }

    pub fn downloaded_layer(&mut self, id: &str, total: u64) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Downloaded;
            entry.indication = OciProgressIndication::Completed {
                message: None,
                total: Some(total),
                bytes: true,
            };
        }
    }

    pub fn start_assemble(&mut self) {
        self.phase = OciProgressPhase::Assemble;
        self.indication = OciProgressIndication::Hidden;
    }

    pub fn start_extracting_layer(&mut self, id: &str) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Extracting;
            entry.indication = OciProgressIndication::Spinner { message: None };
        }
    }

    pub fn extracting_layer(&mut self, id: &str, file: &str) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Extracting;
            entry.indication = OciProgressIndication::Spinner {
                message: Some(file.to_string()),
            };
        }
    }

    pub fn extracted_layer(&mut self, id: &str, count: u64, total_size: u64) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Extracted;
            entry.indication = OciProgressIndication::Completed {
                message: Some(format!("{} files", count)),
                total: Some(total_size),
                bytes: true,
            };
        }
    }

    pub fn start_packing(&mut self) {
        self.phase = OciProgressPhase::Pack;
        for layer in self.layers.values_mut() {
            layer.indication = OciProgressIndication::Hidden;
        }
        self.indication = OciProgressIndication::Spinner { message: None };
    }

    pub fn complete(&mut self, size: u64) {
        self.phase = OciProgressPhase::Complete;
        self.indication = OciProgressIndication::Completed {
            message: None,
            total: Some(size),
            bytes: true,
        }
    }
}

#[derive(Clone, Debug)]
pub enum OciProgressPhase {
    Started,
    Resolving,
    Resolved,
    ConfigDownload,
    LayerDownload,
    Assemble,
    Pack,
    Complete,
}

#[derive(Clone, Debug)]
pub enum OciProgressIndication {
    Hidden,

    ProgressBar {
        message: Option<String>,
        current: u64,
        total: u64,
        bytes: bool,
    },

    Spinner {
        message: Option<String>,
    },

    Completed {
        message: Option<String>,
        total: Option<u64>,
        bytes: bool,
    },
}

#[derive(Clone, Debug)]
pub struct OciProgressLayer {
    pub id: String,
    pub phase: OciProgressLayerPhase,
    pub indication: OciProgressIndication,
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
    sender: watch::Sender<OciProgress>,
}

impl OciProgressContext {
    pub fn create() -> (OciProgressContext, watch::Receiver<OciProgress>) {
        let (sender, receiver) = watch::channel(OciProgress::new());
        (OciProgressContext::new(sender), receiver)
    }

    pub fn new(sender: watch::Sender<OciProgress>) -> OciProgressContext {
        OciProgressContext { sender }
    }

    pub fn update(&self, progress: &OciProgress) {
        let _ = self.sender.send(progress.clone());
    }

    pub fn subscribe(&self) -> watch::Receiver<OciProgress> {
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
            while (receiver.changed().await).is_ok() {
                context
                    .sender
                    .send_replace(receiver.borrow_and_update().clone());
            }
        })
    }
}
