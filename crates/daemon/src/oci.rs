use krata::v1::control::{
    PullImageProgress, PullImageProgressLayer, PullImageProgressLayerPhase, PullImageProgressPhase,
};
use krataoci::progress::{OciProgress, OciProgressLayer, OciProgressLayerPhase, OciProgressPhase};

fn convert_oci_layer_progress(layer: OciProgressLayer) -> PullImageProgressLayer {
    PullImageProgressLayer {
        id: layer.id,
        phase: match layer.phase {
            OciProgressLayerPhase::Waiting => PullImageProgressLayerPhase::Waiting,
            OciProgressLayerPhase::Downloading => PullImageProgressLayerPhase::Downloading,
            OciProgressLayerPhase::Downloaded => PullImageProgressLayerPhase::Downloaded,
            OciProgressLayerPhase::Extracting => PullImageProgressLayerPhase::Extracting,
            OciProgressLayerPhase::Extracted => PullImageProgressLayerPhase::Extracted,
        }
        .into(),
        value: layer.value,
        total: layer.total,
    }
}

pub fn convert_oci_progress(oci: OciProgress) -> PullImageProgress {
    PullImageProgress {
        phase: match oci.phase {
            OciProgressPhase::Resolving => PullImageProgressPhase::Resolving,
            OciProgressPhase::Resolved => PullImageProgressPhase::Resolved,
            OciProgressPhase::ConfigAcquire => PullImageProgressPhase::ConfigAcquire,
            OciProgressPhase::LayerAcquire => PullImageProgressPhase::LayerAcquire,
            OciProgressPhase::Packing => PullImageProgressPhase::Packing,
            OciProgressPhase::Complete => PullImageProgressPhase::Complete,
        }
        .into(),
        layers: oci
            .layers
            .into_values()
            .map(convert_oci_layer_progress)
            .collect::<Vec<_>>(),
        value: oci.value,
        total: oci.total,
    }
}
