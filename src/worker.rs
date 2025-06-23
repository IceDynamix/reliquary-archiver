use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;
use std::sync::Arc;

use futures::executor::block_on;
use futures::lock::Mutex;
use iced::futures::channel::{mpsc, oneshot};
use iced::futures::sink::SinkExt;
use iced::stream;
use iced::futures::Stream;
use iced::futures::StreamExt;
use iced::futures::select;
use iced::Subscription;
use iced::Task;
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
use tracing::info;
use tracing::instrument;
use tracing::warn;

use crate::capture;
use crate::websocket::start_websocket_server;

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready(mpsc::Sender<WorkerCommand>),
    ExportEvent(OptimizerEvent),
    Metric(SnifferMetric),
}

pub enum WorkerCommand {
    Abort,
    MakeExport(oneshot::Sender<Option<Export>>),
    ProcessRecorded(std::path::PathBuf),
}

pub type WorkerHandle = mpsc::Sender<WorkerCommand>;

// pub fn archiver_subscription(exporter: Arc<Mutex<OptimizerExporter>>) -> Subscription<WorkerEvent> {
//     Subscription::run(move || archiver_worker(exporter))
// }

struct AbortOnDrop(Arc<AtomicBool>, Option<tokio::task::JoinHandle<()>>);

impl AbortOnDrop {
    pub fn new(
        f: impl FnOnce(Arc<AtomicBool>) -> tokio::task::JoinHandle<()>,
    ) -> Self {
        let abort_signal = Arc::new(AtomicBool::new(false));
        Self(abort_signal.clone(), Some(f(abort_signal)))
    }

    pub fn abort(&self) {
        self.0.store(true, Ordering::Relaxed);
    }
}

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.abort();
        if let Some(handle) = self.1.take() {
            block_on(handle).ok();
        }
    }
}

struct MappedSender<Output, Intermediate> {
    sender: mpsc::Sender<Output>,
    f: fn(Intermediate) -> Output,
}

impl<Output, Intermediate> MappedSender<Output, Intermediate> {
    fn new(sender: mpsc::Sender<Output>, f: fn(Intermediate) -> Output) -> Self {
        Self {
            sender,
            f,
        }
    }

    fn send(&mut self, item: Intermediate) -> futures::sink::Send<'_, mpsc::Sender<Output>, Output> {
        self.sender.send((self.f)(item))
    }
}

// trait MapSender {
//     type Item;

//     fn map<F, O>(self, f: F) -> Map<Self, F, O>
//     where
//         Self: Sized,
//         F: Fn(Self::Item) -> O + 'static;
// }

// struct Map<S, F, O> {
//     sender: S,
//     f: Box<dyn Fn(S::Item) -> O>,
// }

// impl<T> MapSender for mpsc::Sender<T> {
//     type Item = T;

//     fn map<F, O>(self, f: F) -> Map<Self, F, O>
//     where
//         F: Fn(Self::Item) -> O + 'static,
//     {
//         Map {
//             sender: self,
//             f: Box::new(f),
//         }
//     }
// }

// impl<S: MapSender, F, O> Map<S, F, O> {
//     async fn send(&self, item: S::Item) -> Result<(), mpsc::SendError<O>> {
//         self.sender.send((self.f)(item)).await
//     }
// }

#[instrument(skip_all)]
pub fn archiver_worker(exporter: Arc<Mutex<OptimizerExporter>>) -> impl Stream<Item = WorkerEvent> {
    stream::channel(100, |mut output: mpsc::Sender<WorkerEvent>| async move {
        // Create channel
        let (sender, mut receiver) = mpsc::channel(100);

        let sniffer = GameSniffer::new().set_initial_keys(get_database().keys.clone());
        // let exporter = OptimizerExporter::new();

        let (_, mut rx) = exporter.lock().await.subscribe();

        let (mut recorded_tx, recorded_rx) = mpsc::channel(100);
        // let (metric_tx, mut metric_rx) = mpsc::channel(100);

        let abort_signal = { // Need to spawn a real thread since the packet capture is blocking
            let exporter = exporter.clone();
            let output = output.clone();

            AbortOnDrop::new(move |abort_signal| {
                tokio::spawn(live_capture(
                    abort_signal, 
                    exporter, 
                    sniffer, 
                    MappedSender::new(output, |metric| WorkerEvent::Metric(metric)),
                    recorded_rx
                ))
            })
        };

        output.send(WorkerEvent::Ready(sender)).await.expect("Worker Stream was closed before ready state?");

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
                        WorkerCommand::ProcessRecorded(pcap_path) => {
                            let packets = capture_from_pcap(pcap_path);
                            info!("processing {} packets", packets.len());
                            recorded_tx.send_all(&mut futures::stream::iter(packets.into_iter().map(Ok))).await.ok();
                            info!("processed packets");
                        }
                    }
                }

                // metric = metric_rx.select_next_some() => {
                //     output.send(WorkerEvent::Metric(metric)).await.ok();
                // }

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
fn capture_from_pcap(
    pcap_path: std::path::PathBuf,
) -> Vec<capture::Packet> {
    use crate::capture::PCAP_FILTER;

    info!("Capturing from pcap file: {}", pcap_path.display());
    let mut capture = pcap::Capture::from_file(&pcap_path).expect("could not read pcap file");
    capture.filter(PCAP_FILTER, false).unwrap();

    let mut packets = Vec::new();
    while let Ok(packet) = capture.next_packet() {
        packets.push(capture::Packet {
            data: packet.data.to_vec(),
        });
    }

    packets
}

#[instrument(skip_all)]
async fn live_capture<E: Exporter>(
    abort_signal: Arc<AtomicBool>, 
    exporter: Arc<Mutex<E>>, 
    mut sniffer: GameSniffer,
    mut metric_tx: MappedSender<WorkerEvent, SnifferMetric>,
    mut recorded_rx: mpsc::Receiver<capture::Packet>,
) {
    let live_rx = {
        #[cfg(feature = "pcap")] {
            capture::listen_on_all(capture::pcap::PcapBackend, abort_signal.clone())
        }

        #[cfg(all(not(feature = "pcap"), feature = "pktmon"))] {
            capture::listen_on_all(capture::pktmon::PktmonBackend, abort_signal.clone())
        }
    };

    let (mut live_rx, join_handles) = live_rx.expect("Failed to start packet capture");

    // #[cfg(feature = "stream")]
    // let streaming = args.stream;

    // #[cfg(not(feature = "stream"))]
    // let streaming = false;

    info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");
    // if !streaming {
    //     info!("listening with a timeout of {} seconds...", args.timeout);
    // }

    'recv: loop {
        let packet = select! {
            data = live_rx.next() => match data {
                Some(data) => data,
                None => break 'recv,
            },

            data = recorded_rx.select_next_some() => data,

            complete => break 'recv,
        };

        metric_tx.send(SnifferMetric::NetworkPacketReceived).await.ok();

        match sniffer.receive_packet(packet.data) {
            Ok(packets) => {
                metric_tx.send(SnifferMetric::GameCommandsReceived(packets.len())).await.ok();

                for packet in packets {
                    match packet {
                        GamePacket::Connection(c) => {
                            match c {
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
                            }
                        }
                        GamePacket::Commands(command) => {
                            match command {
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
                            }
                        }
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

    abort_signal.store(true, Ordering::Relaxed);

    #[cfg(target_os = "linux")] {
        // Detach join handles on linux since pcap timeout will not fire if no packets are received on some interface
        drop(join_handles);
    }

    // TODO: determine why pcap timeout is not working on linux, so that we can gracefully exit
    #[cfg(not(target_os = "linux"))] {
        for handle in join_handles {
            // TODO: spawn_blocking?
            handle.join().expect("Failed to join capture thread");
        }
    }
}
