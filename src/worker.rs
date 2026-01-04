use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::executor::block_on;
use futures::lock::Mutex;
use futures::select;
use futures::sink::SinkExt;
use futures::stream;
use futures::stream::FusedStream;
use futures::FutureExt;
use futures::Stream;
use futures::StreamExt;
use reliquary::network::command::command_id::PlayerLoginFinishScRsp;
use reliquary::network::command::command_id::PlayerLoginScRsp;
use reliquary::network::command::GameCommand;
use reliquary::network::command::GameCommandError;
use reliquary::network::ConnectionPacket;
use reliquary::network::GamePacket;
use reliquary::network::GameSniffer;
use reliquary::network::NetworkError;
use reliquary_archiver::export::database::get_database;
use reliquary_archiver::export::database::Database;
use reliquary_archiver::export::fribbels::Export;
use reliquary_archiver::export::fribbels::OptimizerEvent;
use reliquary_archiver::export::fribbels::OptimizerExporter;
use reliquary_archiver::export::Exporter;
use tokio::pin;
use tracing::info;
use tracing::instrument;
use tracing::warn;

use crate::capture;

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready(mpsc::Sender<WorkerCommand>),
    ExportEvent(OptimizerEvent),
    Metric(SnifferMetric),
}

pub enum WorkerCommand {
    Abort,
    MakeExport(oneshot::Sender<Option<Export>>),

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
pub fn archiver_worker(exporter: Arc<Mutex<OptimizerExporter>>) -> impl Stream<Item = WorkerEvent> {
    stream_channel(100, |mut output: mpsc::Sender<WorkerEvent>| async move {
        let (sender, mut receiver) = mpsc::channel(100);

        let sniffer = GameSniffer::new().set_initial_keys(get_database().keys.clone());

        let (_, mut rx) = exporter.lock().await.subscribe();

        let (mut recorded_tx, recorded_rx) = mpsc::channel(100);

        let abort_signal = {
            let exporter = exporter.clone();
            let output = output.clone();

            tokio::spawn(live_capture(
                exporter,
                sniffer,
                MappedSender::new(output, |metric| WorkerEvent::Metric(metric)),
                recorded_rx,
            ))
        };

        output
            .send(WorkerEvent::Ready(sender.clone()))
            .await
            .expect("Worker Stream was closed before ready state?");

        loop {
            tokio::select! {
                cmd = receiver.select_next_some() => {
                    match cmd {
                        WorkerCommand::Abort => {
                            abort_signal.abort();
                        }
                        WorkerCommand::MakeExport(sender) => {
                            let export = exporter.lock().await.export();
                            sender.send(export).ok();
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

                event = rx.recv() => {
                    match event {
                        Ok(event) => {
                            output.send(WorkerEvent::ExportEvent(event)).await.unwrap(); // TODO: handle error
                        }
                        Err(e) => {
                            warn!(%e);
                            break;
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

    use crate::capture::PCAP_FILTER;

    info!("Capturing from pcap file: {}", pcap_path.display());
    let mut capture = pcap::Capture::from_file(&pcap_path).expect("could not read pcap file");
    capture.filter(PCAP_FILTER, false).unwrap();

    let mut hasher = DefaultHasher::new();
    hasher.write(pcap_path.display().to_string().as_bytes());
    let source_id = hasher.finish();

    let mut packets = Vec::new();
    while let Ok(packet) = capture.next_packet() {
        packets.push(capture::Packet {
            source_id,
            data: packet.data.to_vec(),
        });
    }

    packets
}

#[instrument(skip_all)]
async fn live_capture<E: Exporter>(
    exporter: Arc<Mutex<E>>,
    mut sniffer: GameSniffer,
    mut metric_tx: MappedSender<WorkerEvent, SnifferMetric>,
    mut recorded_rx: mpsc::Receiver<capture::Packet>,
) {
    // Outer loop to restart capture when it exits
    loop {
        let live_rx = {
            let result = {
                #[cfg(feature = "pcap")]
                {
                    capture::listen_on_all(capture::pcap::PcapBackend)
                }

                #[cfg(all(not(feature = "pcap"), feature = "pktmon"))]
                {
                    capture::listen_on_all(capture::pktmon::PktmonBackend)
                }
            };

            match result.map_err(|e| e.to_string()) {
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
                                ConnectionPacket::HandshakeEstablished => {
                                    info!("detected connection established");
                                    metric_tx.send(SnifferMetric::ConnectionEstablished).await.ok();

                                    if cfg!(all(feature = "pcap", windows)) {
                                        info!("If the program gets stuck at this point for longer than 10 seconds, please try the pktmon release from https://github.com/IceDynamix/reliquary-archiver/releases/latest");
                                    }
                                }
                                ConnectionPacket::Disconnected => {
                                    info!("detected connection disconnected");
                                    metric_tx.send(SnifferMetric::ConnectionDisconnected).await.ok();
                                }
                                _ => {}
                            },
                            GamePacket::Commands(command) => match command {
                                Ok(command) => {
                                    if command.command_id == PlayerLoginScRsp {
                                        info!("detected login start");
                                    }

                                    exporter.lock().await.read_command(command);
                                }
                                Err(e) => {
                                    warn!(%e);
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
    }
}
