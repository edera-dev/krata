use krata::v1::control::{
    OciProgressEvent, OciProgressEventLayer, OciProgressEventLayerPhase, OciProgressEventPhase,
};
use krataoci::progress::{OciProgress, OciProgressLayer, OciProgressLayerPhase, OciProgressPhase};

fn convert_oci_layer_progress(layer: OciProgressLayer) -> OciProgressEventLayer {
    OciProgressEventLayer {
        id: layer.id,
        phase: match layer.phase {
            OciProgressLayerPhase::Waiting => OciProgressEventLayerPhase::Waiting,
            OciProgressLayerPhase::Downloading => OciProgressEventLayerPhase::Downloading,
            OciProgressLayerPhase::Downloaded => OciProgressEventLayerPhase::Downloaded,
            OciProgressLayerPhase::Extracting => OciProgressEventLayerPhase::Extracting,
            OciProgressLayerPhase::Extracted => OciProgressEventLayerPhase::Extracted,
        }
        .into(),
        progress: layer.progress,
    }
}

pub fn convert_oci_progress(oci: OciProgress) -> OciProgressEvent {
    OciProgressEvent {
        guest_id: oci.id,
        phase: match oci.phase {
            OciProgressPhase::Resolving => OciProgressEventPhase::Resolving,
            OciProgressPhase::Resolved => OciProgressEventPhase::Resolved,
            OciProgressPhase::ConfigAcquire => OciProgressEventPhase::ConfigAcquire,
            OciProgressPhase::LayerAcquire => OciProgressEventPhase::LayerAcquire,
            OciProgressPhase::Packing => OciProgressEventPhase::Packing,
            OciProgressPhase::Complete => OciProgressEventPhase::Complete,
        }
        .into(),
        layers: oci
            .layers
            .into_values()
            .map(convert_oci_layer_progress)
            .collect::<Vec<_>>(),
        progress: oci.progress,
    }
}
