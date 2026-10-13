//! Headless mode: capture until the export is complete, then write it to a file.

use std::error::Error;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use chrono::Local;
use futures::lock::Mutex;
use futures::{FutureExt, StreamExt, future, select};
use reliquary_archiver::export::fribbels::Export;
use tokio::pin;
use tracing::{error, info, instrument, warn};

use crate::Args;
use crate::accounts::MultiAccountManager;
use crate::capture::{self, SourceSpec};
use crate::session::{CaptureSession, SessionEvent};

pub async fn run(args: Args) {
    let source = source_from_args(&args);

    match capture(&args, &source).await {
        Some(export) => write_output(args.output, &export),
        None => warn!("skipped writing output"),
    }

    if let Some(log_path) = args.log_path {
        info!("wrote logs to {}", log_path.display());
    }
}

fn source_from_args(args: &Args) -> SourceSpec {
    #[cfg(feature = "pcap")]
    if let Some(path) = &args.pcap {
        return SourceSpec::PcapFile(path.clone());
    }

    #[cfg(feature = "pktmon")]
    if let Some(path) = &args.etl {
        return SourceSpec::EtlFile(path.clone());
    }

    SourceSpec::Live
}

#[instrument(skip_all)]
async fn capture(args: &Args, source: &SourceSpec) -> Option<Export> {
    let manager = Arc::new(Mutex::new(MultiAccountManager::new()));

    // Recorded captures are read from start to end, streaming only makes sense for live traffic
    #[cfg(feature = "stream")]
    let selected_account_tx = (args.stream && source.is_live()).then(|| {
        use crate::websocket::{PortSource, start_websocket_server};

        let (tx, rx) = tokio::sync::watch::channel::<Option<u32>>(None);

        tokio::spawn(start_websocket_server(PortSource::Fixed(args.websocket_port), manager.clone(), rx));

        info!("WebSocket server starting on port {}...", args.websocket_port);
        tx
    });

    #[cfg(feature = "stream")]
    let streaming = selected_account_tx.is_some();

    #[cfg(not(feature = "stream"))]
    let streaming = false;

    let mut packets = capture::open_source(source).expect("Failed to start packet capture").fuse();

    let timeout = if !source.is_live() {
        None
    } else if streaming {
        info!("WebSocket streaming enabled - capture will run until manually stopped");
        None
    } else {
        info!("listening with a timeout of {} seconds...", args.timeout);
        Some(Duration::from_secs(args.timeout))
    };

    if source.is_live() {
        info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");
    }

    pin!(
        let timeout_future = maybe_timeout(timeout).fuse();
    );

    let mut session = CaptureSession::new(manager, true);

    'recv: loop {
        let packet = select! {
            packet = packets.next() => match packet {
                Some(Ok(packet)) => packet,
                Some(Err(e)) => {
                    warn!(%e);
                    break 'recv;
                }
                None => break 'recv,
            },

            _ = timeout_future => break 'recv,
        };

        for event in session.handle_packet(packet).await {
            match event {
                SessionEvent::AccountSeen { uid } => {
                    #[cfg(feature = "stream")]
                    if let Some(tx) = &selected_account_tx {
                        tx.send(Some(uid)).ok();
                        info!(uid, "Auto-selected account for WebSocket streaming");
                    }

                    #[cfg(not(feature = "stream"))]
                    let _ = uid;
                }
                SessionEvent::LoginFinished if !streaming => {
                    info!("detected login end, assume initialization is finished");
                    break 'recv;
                }
                SessionEvent::DecryptionKeyMissing => {
                    warn!(
                        "This game version is not supported yet. It usually takes a few days for Reliquary Archiver to get updated for new versions. Please try again at a later point."
                    );
                    break 'recv;
                }
                SessionEvent::Fatal => break 'recv,
                _ => {}
            }
        }

        // Check if initialized for early exit in non-streaming mode
        if !streaming && session.is_initialized().await {
            info!("retrieved all relevant packets, stop listening");
            break 'recv;
        }
    }

    session.export().await
}

async fn maybe_timeout(timeout: Option<Duration>) {
    if let Some(timeout) = timeout {
        tokio::time::sleep(timeout).await;
    } else {
        future::pending::<()>().await;
    }
}

fn write_output(output: Option<PathBuf>, export: &Export) {
    let default_name = Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string();
    let mut output_file = output.unwrap_or_else(|| PathBuf::from(&default_name));

    info!("exporting collected data");
    loop {
        match write_json(&output_file, export) {
            Ok(()) => {
                let written_to = output_file.canonicalize().unwrap_or(output_file);
                info!("wrote output to {}", written_to.display());
                return;
            }
            Err(e) => {
                error!("Failed to write to {}: {}", output_file.display(), e);

                match pick_alternative_path(&default_name) {
                    Some(new_path) => output_file = new_path,
                    None => {
                        error!("No alternative path selected, aborting write");
                        return;
                    }
                }
            }
        }
    }
}

fn write_json(path: &Path, export: &Export) -> Result<(), Box<dyn Error>> {
    let file = File::create(path)?;
    serde_json::to_writer_pretty(&file, export)?;
    Ok(())
}

#[cfg(feature = "gui")]
fn pick_alternative_path(file_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .set_title("Select output file location")
        .set_file_name(file_name)
        .add_filter("JSON files", &["json"])
        .save_file()
}

#[cfg(not(feature = "gui"))]
fn pick_alternative_path(_file_name: &str) -> Option<PathBuf> {
    None
}
