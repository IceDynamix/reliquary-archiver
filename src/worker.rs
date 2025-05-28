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
use reliquary::network::ConnectionPacket;
use reliquary::network::GamePacket;
use reliquary::network::GameSniffer;
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
    Event(OptimizerEvent),
}

pub enum WorkerCommand {
    Abort,
    MakeExport(oneshot::Sender<Option<Export>>),
}

pub type WorkerHandle = mpsc::Sender<WorkerCommand>;

// pub fn archiver_subscription(exporter: Arc<Mutex<OptimizerExporter>>) -> Subscription<WorkerEvent> {
//     Subscription::run(move || archiver_worker(exporter))
// }

struct AbortOnDrop(Arc<AtomicBool>, Option<std::thread::JoinHandle<()>>);

impl AbortOnDrop {
    pub fn new(
        f: impl FnOnce(Arc<AtomicBool>) -> std::thread::JoinHandle<()>,
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
            handle.join().unwrap();
        }
    }
}

#[instrument(skip_all)]
pub fn archiver_worker(exporter: Arc<Mutex<OptimizerExporter>>) -> impl Stream<Item = WorkerEvent> {
    stream::channel(100, |mut output: mpsc::Sender<WorkerEvent>| async move {
        // Create channel
        let (sender, mut receiver) = mpsc::channel(100);

        let sniffer = GameSniffer::new().set_initial_keys(get_database().keys.clone());
        // let exporter = OptimizerExporter::new();

        let (_, mut rx) = exporter.lock().await.subscribe();

        let abort_signal = { // Need to spawn a real thread since the packet capture is blocking
            let exporter = exporter.clone();

            AbortOnDrop::new(move |abort_signal| {
                std::thread::spawn(move || {
                    live_capture(abort_signal, exporter, sniffer)
                })
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
                    }
                }

                event = rx.recv() => {
                    match event {
                        Ok(event) => {
                            output.send(WorkerEvent::Event(event)).await.unwrap();
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

#[instrument(skip_all)]
fn live_capture<E: Exporter>(abort_signal: Arc<AtomicBool>, exporter: Arc<Mutex<E>>, mut sniffer: GameSniffer) {
    let rx = {
        #[cfg(feature = "pcap")] {
            capture::listen_on_all(capture::pcap::PcapBackend, abort_signal.clone())
        }

        #[cfg(all(not(feature = "pcap"), feature = "pktmon"))] {
            capture::listen_on_all(capture::pktmon::PktmonBackend, abort_signal.clone())
        }
    };

    let (rx, join_handles) = rx.expect("Failed to start packet capture");

    // #[cfg(feature = "stream")]
    // let streaming = args.stream;

    // #[cfg(not(feature = "stream"))]
    // let streaming = false;

    info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");
    // if !streaming {
    //     info!("listening with a timeout of {} seconds...", args.timeout);
    // }

    'recv: loop {
        match rx.recv() {
            Ok(packet) => {
                match sniffer.receive_packet(packet.data) {
                    Ok(packets) => {
                        for packet in packets {
                            match packet {
                                GamePacket::Connection(c) => {
                                    match c {
                                        ConnectionPacket::HandshakeEstablished => {
                                            info!("detected connection established");

                                            if cfg!(all(feature = "pcap", windows)) {
                                                info!("If the program gets stuck at this point for longer than 10 seconds, please try the pktmon release from https://github.com/IceDynamix/reliquary-archiver/releases/latest");
                                            }
                                        }
                                        ConnectionPacket::Disconnected => {
                                            info!("detected connection disconnected");
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

                                            block_on(exporter.lock()).read_command(command);
                                        }
                                        Err(e) => {
                                            warn!(%e);
                                            break 'recv;
                                        }
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(%e);
                        break 'recv;
                    }
                }
            }
            Err(e) => {
                warn!(%e);
                break 'recv;
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
            handle.join().expect("Failed to join capture thread");
        }
    }
}
