use std::collections::HashMap;

use anyhow::{anyhow, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use krata::v1::control::{PullImageProgressLayerPhase, PullImageProgressPhase, PullImageReply};
use tokio_stream::StreamExt;
use tonic::Streaming;

pub async fn pull_interactive_progress(
    mut stream: Streaming<PullImageReply>,
) -> Result<PullImageReply> {
    let mut multi_progress: Option<(MultiProgress, HashMap<String, ProgressBar>)> = None;

    while let Some(reply) = stream.next().await {
        let reply = reply?;

        if reply.progress.is_none() && !reply.digest.is_empty() {
            return Ok(reply);
        }

        let Some(oci) = reply.progress else {
            continue;
        };

        if multi_progress.is_none() {
            multi_progress = Some((MultiProgress::new(), HashMap::new()));
        }

        let Some((multi_progress, progresses)) = multi_progress.as_mut() else {
            continue;
        };

        match oci.phase() {
            PullImageProgressPhase::Resolved
            | PullImageProgressPhase::ConfigAcquire
            | PullImageProgressPhase::LayerAcquire => {
                if progresses.is_empty() && !oci.layers.is_empty() {
                    for layer in &oci.layers {
                        let bar = ProgressBar::new(layer.total);
                        bar.set_style(ProgressStyle::with_template("{msg} {wide_bar}").unwrap());
                        progresses.insert(layer.id.clone(), bar.clone());
                        multi_progress.add(bar);
                    }
                }

                for layer in oci.layers {
                    let Some(progress) = progresses.get_mut(&layer.id) else {
                        continue;
                    };

                    let phase = match layer.phase() {
                        PullImageProgressLayerPhase::Waiting => "waiting",
                        PullImageProgressLayerPhase::Downloading => "downloading",
                        PullImageProgressLayerPhase::Downloaded => "downloaded",
                        PullImageProgressLayerPhase::Extracting => "extracting",
                        PullImageProgressLayerPhase::Extracted => "extracted",
                        _ => "unknown",
                    };

                    let simple = if let Some((_, hash)) = layer.id.split_once(':') {
                        hash
                    } else {
                        "unknown"
                    };
                    let simple = if simple.len() > 10 {
                        &simple[0..10]
                    } else {
                        simple
                    };
                    let message = format!(
                        "{:width$} {:phwidth$}",
                        simple,
                        phase,
                        width = 10,
                        phwidth = 11
                    );

                    if message != progress.message() {
                        progress.set_message(message);
                    }

                    progress.update(|state| {
                        state.set_len(layer.total);
                        state.set_pos(layer.value);
                    });
                }
            }

            PullImageProgressPhase::Packing => {
                for (key, bar) in &mut *progresses {
                    if key == "packing" {
                        continue;
                    }
                    bar.finish_and_clear();
                    multi_progress.remove(bar);
                }
                progresses.retain(|k, _| k == "packing");
                if progresses.is_empty() {
                    let progress = ProgressBar::new(100);
                    progress.set_message("packing    ");
                    progress.set_style(ProgressStyle::with_template("{msg} {wide_bar}").unwrap());
                    progresses.insert("packing".to_string(), progress);
                }
                let Some(progress) = progresses.get("packing") else {
                    continue;
                };

                progress.update(|state| {
                    state.set_len(oci.total);
                    state.set_pos(oci.value);
                });
            }

            _ => {}
        }
    }
    Err(anyhow!("never received final reply for image pull"))
}
