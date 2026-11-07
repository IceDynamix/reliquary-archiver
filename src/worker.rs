use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::executor::block_on;
use futures::lock::Mutex;
use futures::sink::SinkExt;
use futures::stream::FusedStream;
use futures::{FutureExt, Stream, StreamExt, select, stream};
use reliquary::network::command::command_id::{PlayerGetTokenScRsp, PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp as PlayerGetTokenScRspProto;
use reliquary::network::command::{GameCommand, GameCommandError};
use reliquary::network::{ConnectionPacket, GamePacket, GameSniffer, NetworkError};
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::database::{Database, get_database};
use reliquary_archiver::export::fribbels::{Export, OptimizerEvent, OptimizerExporter};
use tokio::pin;
use tracing::{info, instrument, warn};

pub use crate::accounts::{AccountEvent, MultiAccountManager};
use crate::capture;

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready(mpsc::Sender<WorkerCommand>),
    AccountDiscovered { uid: u32 },
    AccountReconnected { uid: u32 },
    ExportEvent { uid: u32, event: OptimizerEvent },
    Metric(SnifferMetric),
}

pub enum WorkerCommand {
    Abort,
    MakeExport {
        uid: Option<u32>, // None = first account
        sender: oneshot::Sender<Option<Export>>,
    },

    #[cfg(feature = "pcap")]
    ProcessRecorded(std::path::PathBuf),
}

pub type WorkerHandle = mpsc::Sender<WorkerCommand>;

struct MappedSender<Output, Intermediate> {
    sender: mpsc::Sender<Output>,
    f: fn(Intermediate) -> Output,
}

impl<Output, Intermediate> MappedSender<Output, Intermediate> {
    fn new(sender: mpsc::Sender<Output>, f: fn(Intermediate) -> Output) -> Self {
        Self { sender, f }
    }

    fn send(&mut self, item: Intermediate) -> futures::sink::Send<'_, mpsc::Sender<Output>, Output> {
        self.sender.send((self.f)(item))
    }
}

/// Creates a new [`Stream`] that produces the items sent from a [`Future`]
/// to the [`mpsc::Sender`] provided to the closure.
///
/// This is a more ergonomic [`stream::unfold`], which allows you to go
/// from the "world of futures" to the "world of streams" by simply looping
/// and publishing to an async channel from inside a [`Future`].
pub fn stream_channel<T>(size: usize, f: impl AsyncFnOnce(mpsc::Sender<T>)) -> impl Stream<Item = T> {
    let (sender, receiver) = mpsc::channel(size);

    let runner = stream::once(f(sender)).filter_map(|_| async { None });

    stream::select(receiver, runner)
}

#[instrument(skip_all)]
pub fn archiver_worker(manager: Arc<Mutex<MultiAccountManager>>) -> impl Stream<Item = WorkerEvent> {
    stream_channel(100, |mut output: mpsc::Sender<WorkerEvent>| async move {
        let (sender, mut receiver) = mpsc::channel(100);

        let database = get_database();
        let sniffer = GameSniffer::new();

        let (mut recorded_tx, recorded_rx) = mpsc::channel(100);

        let abort_signal = {
            let manager = manager.clone();
            let output = output.clone();

            tokio::spawn(live_capture(
                manager,
                sniffer,
                MappedSender::new(output, WorkerEvent::Metric),
                recorded_rx,
            ))
        };

        // Subscribe to manager's unified event channel
        let mut manager_rx = manager.lock().await.subscribe();

        output
            .send(WorkerEvent::Ready(sender.clone()))
            .await
            .expect("Worker Stream was closed before ready state?");

        loop {
            tokio::select! {
                // Forward events from manager's unified event channel
                Ok(event) = manager_rx.recv() => {
                    match event {
                        // The capture loop already reports every login as AccountDiscovered
                        AccountEvent::Discovered { .. } => {}
                        AccountEvent::Reconnected { uid } => {
                            output.send(WorkerEvent::AccountReconnected { uid }).await.ok();
                        }
                        AccountEvent::ExporterEvent { uid, event } => {
                            output.send(WorkerEvent::ExportEvent { uid, event }).await.ok();
                        }
                    }
                }

                cmd = receiver.select_next_some() => {
                    match cmd {
                        WorkerCommand::Abort => {
                            abort_signal.abort();
                        }
                        WorkerCommand::MakeExport { uid, sender: response_sender } => {
                            let export = if let Some(target_uid) = uid {
                                // Export specific account
                                let exporter_opt = manager.lock().await.get_account_exporter(target_uid);
                                if let Some(exporter) = exporter_opt
                                && let exporter = exporter.lock().await
                                && exporter.is_initialized() {
                                    exporter.export()
                                } else {
                                    None
                                }
                            } else {
                                // Export first account
                                let accounts = manager.lock().await.get_all_accounts();
                                if let Some((_, exporter)) = accounts.first()
                                && let exporter = exporter.lock().await
                                && exporter.is_initialized() {
                                    exporter.export()
                                } else {
                                    None
                                }
                            };
                            response_sender.send(export).ok();
                        }

                        #[cfg(feature = "pcap")]
                        WorkerCommand::ProcessRecorded(pcap_path) => {
                            let packets = capture_from_pcap(pcap_path);
                            info!("processing {} packets", packets.len());
                            recorded_tx.send_all(&mut futures::stream::iter(packets.into_iter().map(Ok))).await.ok();
                            info!("processed packets");
                        }
                    }
                }
            }
        }
    })
}

#[derive(Debug, Clone)]
pub enum SnifferMetric {
    ConnectionEstablished,
    ConnectionDisconnected,
    NetworkPacketReceived,
    GameCommandsReceived(usize),
    DecryptionKeyMissing,
    NetworkError,
}

#[instrument(skip_all)]
#[cfg(feature = "pcap")]
fn capture_from_pcap(pcap_path: std::path::PathBuf) -> Vec<capture::Packet> {
    use std::hash::{DefaultHasher, Hasher};

    use crate::capture::{PCAP_FILTER, normalize_offline_pcap_payload};

    info!("Capturing from pcap file: {}", pcap_path.display());
    let mut capture = pcap::Capture::from_file(&pcap_path).expect("could not read pcap file");
    let linktype = capture.get_datalink();
    capture.filter(PCAP_FILTER, false).unwrap();

    let mut hasher = DefaultHasher::new();
    hasher.write(pcap_path.display().to_string().as_bytes());
    let source_id = hasher.finish();

    let mut packets = Vec::new();
    while let Ok(packet) = capture.next_packet() {
        let payload = match normalize_offline_pcap_payload(linktype, packet.data) {
            Ok(payload) => payload,
            Err(_) => continue,
        };
        packets.push(capture::Packet {
            source_id,
            data: payload,
        });
    }

    packets
}

#[instrument(skip_all)]
async fn live_capture(
    manager: Arc<Mutex<MultiAccountManager>>,
    mut sniffer: GameSniffer,
    mut metric_tx: MappedSender<WorkerEvent, SnifferMetric>,
    mut recorded_rx: mpsc::Receiver<capture::Packet>,
) {
    // Outer loop to restart capture when it exits
    loop {
        let live_rx = {
            match capture::open_source(&capture::SourceSpec::Live).map_err(|e| e.to_string()) {
                Ok(rx) => rx,
                Err(err_msg) => {
                    warn!(error = %err_msg, "Failed to start packet capture, retrying in 1 second...");
                    tokio::time::sleep(Duration::from_secs(1)).await;
                    continue;
                }
            }
        };
        let mut live_rx = live_rx.fuse();

        info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");

        'recv: loop {
            // We have to drop the Err before we cross an await point since StdErr is not Send
            let packet = {
                let packet = select! {
                    data = live_rx.next() => match data {
                        Some(data) => data,
                        None => {
                            warn!("live capture stream ended unexpectedly");
                            break 'recv;
                        }
                    },

                    data = recorded_rx.select_next_some() => Ok(data),
                };

                match packet {
                    Ok(packet) => packet,
                    Err(e) => {
                        warn!(%e);
                        continue;
                    }
                }
            };

            metric_tx.send(SnifferMetric::NetworkPacketReceived).await.ok();

            match sniffer.receive_packet(packet.data) {
                Ok(packets) => {
                    metric_tx.send(SnifferMetric::GameCommandsReceived(packets.len())).await.ok();

                    for packet in packets {
                        match packet {
                            GamePacket::Connection(c) => match c {
                                ConnectionPacket::HandshakeEstablished { conv_id } => {
                                    info!(conv_id, "detected connection established");
                                    metric_tx.send(SnifferMetric::ConnectionEstablished).await.ok();

                                    if cfg!(all(feature = "pcap", windows)) {
                                        info!(
                                            "If the program gets stuck at this point for longer than 10 seconds, please try the pktmon release from https://github.com/IceDynamix/reliquary-archiver/releases/latest"
                                        );
                                    }
                                }
                                ConnectionPacket::Disconnected => {
                                    info!("detected connection disconnected");
                                    metric_tx.send(SnifferMetric::ConnectionDisconnected).await.ok();
                                }
                                _ => {}
                            },
                            GamePacket::Commands { conv_id, result } => match result {
                                Ok(command) => {
                                    if command.command_id == PlayerLoginScRsp {
                                        info!(conv_id, "detected login start");
                                    }

                                    // Route command to the correct exporter
                                    let exporter = {
                                        let mut mgr = manager.lock().await;
                                        mgr.get_or_create_exporter(conv_id)
                                    };

                                    // Check if this is a UID discovery packet and register it if so
                                    if command.command_id == PlayerGetTokenScRsp {
                                        if let Ok(token_rsp) = command.parse_proto::<PlayerGetTokenScRspProto>() {
                                            let uid = token_rsp.uid;
                                            let mut mgr = manager.lock().await;
                                            let is_new = mgr.register_uid(conv_id, uid);

                                            metric_tx.sender.send(WorkerEvent::AccountDiscovered { uid }).await.ok();
                                        }
                                    }

                                    exporter.lock().await.read_command(command);
                                }
                                Err(e) => {
                                    warn!(conv_id, %e);
                                    if let GameCommandError::DecryptionKeyMissing = e {
                                        metric_tx.send(SnifferMetric::DecryptionKeyMissing).await.ok();
                                    } else {
                                        metric_tx.send(SnifferMetric::NetworkError).await.ok();
                                    }
                                }
                            },
                        }
                    }
                }
                Err(e) => {
                    warn!(%e);
                    if let NetworkError::GameCommand(GameCommandError::DecryptionKeyMissing) = e {
                        metric_tx.send(SnifferMetric::DecryptionKeyMissing).await.ok();
                    } else {
                        metric_tx.send(SnifferMetric::NetworkError).await.ok();
                    }
                }
            }
        }

        info!("capture ended, restarting...");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
