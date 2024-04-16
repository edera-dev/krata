use krata::v1::control::{
    image_progress_indication::Indication, ImageProgress, ImageProgressIndication,
    ImageProgressIndicationBar, ImageProgressIndicationCompleted, ImageProgressIndicationHidden,
    ImageProgressIndicationSpinner, ImageProgressLayer, ImageProgressLayerPhase,
    ImageProgressPhase,
};
use krataoci::progress::{
    OciProgress, OciProgressIndication, OciProgressLayer, OciProgressLayerPhase, OciProgressPhase,
};

fn convert_oci_progress_indication(indication: OciProgressIndication) -> ImageProgressIndication {
    ImageProgressIndication {
        indication: Some(match indication {
            OciProgressIndication::Hidden => Indication::Hidden(ImageProgressIndicationHidden {}),
            OciProgressIndication::ProgressBar {
                message,
                current,
                total,
                bytes,
            } => Indication::Bar(ImageProgressIndicationBar {
                message: message.unwrap_or_default(),
                current,
                total,
                is_bytes: bytes,
            }),
            OciProgressIndication::Spinner { message } => {
                Indication::Spinner(ImageProgressIndicationSpinner {
                    message: message.unwrap_or_default(),
                })
            }
            OciProgressIndication::Completed {
                message,
                total,
                bytes,
            } => Indication::Completed(ImageProgressIndicationCompleted {
                message: message.unwrap_or_default(),
                total: total.unwrap_or(0),
                is_bytes: bytes,
            }),
        }),
    }
}

fn convert_oci_layer_progress(layer: OciProgressLayer) -> ImageProgressLayer {
    ImageProgressLayer {
        id: layer.id,
        phase: match layer.phase {
            OciProgressLayerPhase::Waiting => ImageProgressLayerPhase::Waiting,
            OciProgressLayerPhase::Downloading => ImageProgressLayerPhase::Downloading,
            OciProgressLayerPhase::Downloaded => ImageProgressLayerPhase::Downloaded,
            OciProgressLayerPhase::Extracting => ImageProgressLayerPhase::Extracting,
            OciProgressLayerPhase::Extracted => ImageProgressLayerPhase::Extracted,
        }
        .into(),
        indication: Some(convert_oci_progress_indication(layer.indication)),
    }
}

pub fn convert_oci_progress(oci: OciProgress) -> ImageProgress {
    ImageProgress {
        phase: match oci.phase {
            OciProgressPhase::Started => ImageProgressPhase::Started,
            OciProgressPhase::Resolving => ImageProgressPhase::Resolving,
            OciProgressPhase::Resolved => ImageProgressPhase::Resolved,
            OciProgressPhase::ConfigDownload => ImageProgressPhase::ConfigDownload,
            OciProgressPhase::LayerDownload => ImageProgressPhase::LayerDownload,
            OciProgressPhase::Assemble => ImageProgressPhase::Assemble,
            OciProgressPhase::Pack => ImageProgressPhase::Pack,
            OciProgressPhase::Complete => ImageProgressPhase::Complete,
        }
        .into(),
        layers: oci
            .layers
            .into_values()
            .map(convert_oci_layer_progress)
            .collect::<Vec<_>>(),
        indication: Some(convert_oci_progress_indication(oci.indication)),
    }
}
