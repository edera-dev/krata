use std::{
    collections::{hash_map::Entry, HashMap},
    time::Duration,
};

use anyhow::{anyhow, Result};
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use krata::v1::control::{
    image_progress_indication::Indication, ImageProgressIndication, ImageProgressLayerPhase,
    ImageProgressPhase, PullImageReply,
};
use tokio_stream::StreamExt;
use tonic::Streaming;

const SPINNER_STRINGS: &[&str] = &[
    "[=                   ]",
    "[ =                  ]",
    "[  =                 ]",
    "[   =                ]",
    "[    =               ]",
    "[     =              ]",
    "[      =             ]",
    "[       =            ]",
    "[        =           ]",
    "[         =          ]",
    "[          =         ]",
    "[           =        ]",
    "[            =       ]",
    "[             =      ]",
    "[              =     ]",
    "[               =    ]",
    "[                =   ]",
    "[                 =  ]",
    "[                  = ]",
    "[                   =]",
    "[====================]",
];

fn progress_bar_for_indication(indication: &ImageProgressIndication) -> Option<ProgressBar> {
    match indication.indication.as_ref() {
        Some(Indication::Hidden(_)) | None => None,
        Some(Indication::Bar(indic)) => {
            let bar = ProgressBar::new(indic.total);
            bar.enable_steady_tick(Duration::from_millis(100));
            Some(bar)
        }
        Some(Indication::Spinner(_)) => {
            let bar = ProgressBar::new_spinner();
            bar.enable_steady_tick(Duration::from_millis(100));
            Some(bar)
        }
        Some(Indication::Completed(indic)) => {
            let bar = ProgressBar::new_spinner();
            bar.enable_steady_tick(Duration::from_millis(100));
            if !indic.message.is_empty() {
                bar.finish_with_message(indic.message.clone());
            } else {
                bar.finish()
            }
            Some(bar)
        }
    }
}

fn configure_for_indication(
    bar: &mut ProgressBar,
    multi_progress: &mut MultiProgress,
    indication: &ImageProgressIndication,
    top_phase: Option<ImageProgressPhase>,
    layer_phase: Option<ImageProgressLayerPhase>,
    layer_id: Option<&str>,
) {
    let prefix = if let Some(phase) = top_phase {
        match phase {
            ImageProgressPhase::Unknown => "unknown",
            ImageProgressPhase::Started => "started",
            ImageProgressPhase::Resolving => "resolving",
            ImageProgressPhase::Resolved => "resolved",
            ImageProgressPhase::ConfigDownload => "downloading",
            ImageProgressPhase::LayerDownload => "downloading",
            ImageProgressPhase::Assemble => "assembling",
            ImageProgressPhase::Pack => "packing",
            ImageProgressPhase::Complete => "complete",
        }
    } else if let Some(phase) = layer_phase {
        match phase {
            ImageProgressLayerPhase::Unknown => "unknown",
            ImageProgressLayerPhase::Waiting => "waiting",
            ImageProgressLayerPhase::Downloading => "downloading",
            ImageProgressLayerPhase::Downloaded => "downloaded",
            ImageProgressLayerPhase::Extracting => "extracting",
            ImageProgressLayerPhase::Extracted => "extracted",
        }
    } else {
        ""
    };
    let prefix = prefix.to_string();

    let id = if let Some(layer_id) = layer_id {
        let hash = if let Some((_, hash)) = layer_id.split_once(':') {
            hash
        } else {
            "unknown"
        };
        let small_hash = if hash.len() > 10 { &hash[0..10] } else { hash };
        Some(format!("{:width$}", small_hash, width = 10))
    } else {
        None
    };

    let prefix = if let Some(id) = id {
        format!("{} {:width$}", id, prefix, width = 11)
    } else {
        format!("           {:width$}", prefix, width = 11)
    };

    match indication.indication.as_ref() {
        Some(Indication::Hidden(_)) | None => {
            multi_progress.remove(bar);
            return;
        }
        Some(Indication::Bar(indic)) => {
            if indic.is_bytes {
                bar.set_style(ProgressStyle::with_template("{prefix} [{bar:20}] {msg} {binary_bytes}/{binary_total_bytes} ({binary_bytes_per_sec}) eta: {eta}").unwrap().progress_chars("=>-"));
            } else {
                bar.set_style(
                    ProgressStyle::with_template(
                        "{prefix} [{bar:20} {msg} {human_pos}/{human_len} ({per_sec}/sec)",
                    )
                    .unwrap()
                    .progress_chars("=>-"),
                );
            }
            bar.set_message(indic.message.clone());
            bar.set_position(indic.current);
            bar.set_length(indic.total);
        }
        Some(Indication::Spinner(indic)) => {
            bar.set_style(
                ProgressStyle::with_template("{prefix} {spinner}  {msg}")
                    .unwrap()
                    .tick_strings(SPINNER_STRINGS),
            );
            bar.set_message(indic.message.clone());
        }
        Some(Indication::Completed(indic)) => {
            if bar.is_finished() {
                return;
            }
            bar.disable_steady_tick();
            bar.set_message(indic.message.clone());
            if indic.total != 0 {
                bar.set_position(indic.total);
                bar.set_length(indic.total);
            }
            if bar.style().get_tick_str(0).contains('=') {
                bar.set_style(
                    ProgressStyle::with_template("{prefix} {spinner}  {msg}")
                        .unwrap()
                        .tick_strings(SPINNER_STRINGS),
                );
                bar.finish_with_message(indic.message.clone());
            } else if indic.is_bytes {
                bar.set_style(
                    ProgressStyle::with_template("{prefix} [{bar:20}] {msg} {binary_total_bytes}")
                        .unwrap()
                        .progress_chars("=>-"),
                );
            } else {
                bar.set_style(
                    ProgressStyle::with_template("{prefix} [{bar:20}] {msg}")
                        .unwrap()
                        .progress_chars("=>-"),
                );
            }
            bar.tick();
            bar.enable_steady_tick(Duration::from_millis(100));
        }
    };

    bar.set_prefix(prefix);
    bar.tick();
}

pub async fn pull_interactive_progress(
    mut stream: Streaming<PullImageReply>,
) -> Result<PullImageReply> {
    let mut multi_progress = MultiProgress::new();
    multi_progress.set_move_cursor(false);
    let mut progresses = HashMap::new();

    while let Some(reply) = stream.next().await {
        let reply = match reply {
            Ok(reply) => reply,
            Err(error) => {
                multi_progress.clear()?;
                return Err(error.into());
            }
        };

        if reply.progress.is_none() && !reply.digest.is_empty() {
            multi_progress.clear()?;
            return Ok(reply);
        }

        let Some(oci) = reply.progress else {
            continue;
        };

        for layer in &oci.layers {
            let Some(ref indication) = layer.indication else {
                continue;
            };

            let bar = match progresses.entry(layer.id.clone()) {
                Entry::Occupied(entry) => Some(entry.into_mut()),

                Entry::Vacant(entry) => {
                    if let Some(bar) = progress_bar_for_indication(indication) {
                        multi_progress.add(bar.clone());
                        Some(entry.insert(bar))
                    } else {
                        None
                    }
                }
            };

            if let Some(bar) = bar {
                configure_for_indication(
                    bar,
                    &mut multi_progress,
                    indication,
                    None,
                    Some(layer.phase()),
                    Some(&layer.id),
                );
            }
        }

        if let Some(ref indication) = oci.indication {
            let bar = match progresses.entry("root".to_string()) {
                Entry::Occupied(entry) => Some(entry.into_mut()),

                Entry::Vacant(entry) => {
                    if let Some(bar) = progress_bar_for_indication(indication) {
                        multi_progress.add(bar.clone());
                        Some(entry.insert(bar))
                    } else {
                        None
                    }
                }
            };

            if let Some(bar) = bar {
                configure_for_indication(
                    bar,
                    &mut multi_progress,
                    indication,
                    Some(oci.phase()),
                    None,
                    None,
                );
            }
        }
    }
    multi_progress.clear()?;
    Err(anyhow!("never received final reply for image pull"))
}
