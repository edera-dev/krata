use std::collections::BTreeMap;

use tokio::sync::broadcast::Sender;

#[derive(Clone, Debug)]
pub struct OciProgress {
    pub id: String,
    pub phase: OciProgressPhase,
    pub layers: BTreeMap<String, OciProgressLayer>,
    pub progress: f64,
}

impl OciProgress {
    pub fn add_layer(&mut self, id: &str) {
        self.layers.insert(
            id.to_string(),
            OciProgressLayer {
                id: id.to_string(),
                phase: OciProgressLayerPhase::Waiting,
                progress: 0.0,
            },
        );
    }

    pub fn downloading_layer(&mut self, id: &str, downloaded: usize, total: usize) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Downloading;
            entry.progress = if total != 0 {
                (downloaded as f64 / total as f64) * 100.0
            } else {
                100.0
            };
        }
    }

    pub fn downloaded_layer(&mut self, id: &str) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Downloaded;
            entry.progress = 100.0;
        }
    }

    pub fn extracting_layer(&mut self, id: &str, extracted: usize, total: usize) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Extracting;
            entry.progress = if total != 0 {
                (extracted as f64 / total as f64) * 100.0
            } else {
                100.0
            };
        }
    }

    pub fn extracted_layer(&mut self, id: &str) {
        if let Some(entry) = self.layers.get_mut(id) {
            entry.phase = OciProgressLayerPhase::Extracted;
            entry.progress = 100.0;
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
    pub progress: f64,
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
    sender: Sender<OciProgress>,
}

impl OciProgressContext {
    pub fn new(sender: Sender<OciProgress>) -> OciProgressContext {
        OciProgressContext { sender }
    }

    pub fn update(&self, progress: &OciProgress) {
        let _ = self.sender.send(progress.clone());
    }
}
