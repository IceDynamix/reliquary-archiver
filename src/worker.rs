use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use futures::channel::{mpsc, oneshot};
use futures::executor::block_on;
use futures::lock::Mutex;
use futures::sink::SinkExt;
use futures::stream::FusedStream;
use futures::{select, stream, FutureExt, Stream, StreamExt};
use reliquary::network::command::command_id::{PlayerGetTokenScRsp, PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp as PlayerGetTokenScRspProto;
use reliquary::network::command::{GameCommand, GameCommandError};
use reliquary::network::{ConnectionPacket, GamePacket, GameSniffer, NetworkError};
use reliquary_archiver::export::database::{get_database, Database};
use reliquary_archiver::export::fribbels::{Export, OptimizerEvent, OptimizerExporter};
use reliquary_archiver::export::Exporter;
use tokio::pin;
use tracing::{info, instrument, warn};

use crate::capture;

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    Ready(mpsc::Sender<WorkerCommand>),
    AccountDiscovered { uid: u32, is_active: bool },
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

/// Manages multiple account exporters, one per conversation.
/// Maps conv_id -> exporter and uid -> conv_id for account tracking.
pub struct MultiAccountManager {
    /// All exporters indexed by conversation ID
    exporters: HashMap<u32, Arc<Mutex<OptimizerExporter>>>,
    
    /// Track which conversation belongs to which account UID
    uid_to_conv: HashMap<u32, u32>,
    
    /// Initial decryption keys shared across all exporters
    initial_keys: HashMap<u32, Vec<u8>>,
}

impl MultiAccountManager {
    pub fn new(initial_keys: HashMap<u32, Vec<u8>>) -> Self {
        Self {
            exporters: HashMap::new(),
            uid_to_conv: HashMap::new(),
            initial_keys,
        }
    }
    
    /// Get or create an exporter for a conversation ID
    pub fn get_or_create_exporter(&mut self, conv_id: u32) -> Arc<Mutex<OptimizerExporter>> {
        self.exporters
            .entry(conv_id)
            .or_insert_with(|| Arc::new(Mutex::new(OptimizerExporter::new())))
            .clone()
    }
    
    /// Register a UID for a conversation. If the UID was previously associated
    /// with a different conversation, delete the old exporter.
    pub fn register_uid(&mut self, conv_id: u32, uid: u32) -> bool {
        let is_new_account = if let Some(&old_conv_id) = self.uid_to_conv.get(&uid) {
            if old_conv_id != conv_id {
                // Same UID reconnecting on a different conversation - delete old exporter
                info!(uid, old_conv = old_conv_id, new_conv = conv_id, "account reconnected, replacing old exporter");
                self.exporters.remove(&old_conv_id);
                self.uid_to_conv.insert(uid, conv_id);
                false // Not a new account, it's a reconnection
            } else {
                // Same conv_id, same UID - no-op
                false
            }
        } else {
            // New UID discovered
            info!(uid, conv_id, "new account discovered");
            self.uid_to_conv.insert(uid, conv_id);
            true
        };
        
        is_new_account
    }
    
    /// Get the exporter for a specific UID
    pub fn get_account_exporter(&self, uid: u32) -> Option<Arc<Mutex<OptimizerExporter>>> {
        self.uid_to_conv
            .get(&uid)
            .and_then(|conv_id| self.exporters.get(conv_id))
            .cloned()
    }
    
    /// Get all accounts (uid, exporter) pairs
    pub fn get_all_accounts(&self) -> Vec<(u32, Arc<Mutex<OptimizerExporter>>)> {
        self.uid_to_conv
            .iter()
            .filter_map(|(uid, conv_id)| {
                self.exporters.get(conv_id).map(|exp| (*uid, exp.clone()))
            })
            .collect()
    }
    
    /// Check if a conversation is still active (has an exporter)
    fn is_conv_active(&self, conv_id: u32) -> bool {
        self.exporters.contains_key(&conv_id)
    }
}

#[instrument(skip_all)]
pub fn archiver_worker(manager: Arc<Mutex<MultiAccountManager>>) -> impl Stream<Item = WorkerEvent> {
    stream_channel(100, |mut output: mpsc::Sender<WorkerEvent>| async move {
        let (sender, mut receiver) = mpsc::channel(100);

        let database = get_database();
        let sniffer = GameSniffer::new().set_initial_keys(database.keys.clone());

        let (mut recorded_tx, recorded_rx) = mpsc::channel(100);
        
        // Track subscriptions: uid -> (exporter Arc for identity check, forwarding task handle)
        let mut subscriptions: HashMap<u32, (Arc<Mutex<OptimizerExporter>>, tokio::task::JoinHandle<()>)> = HashMap::new();
        
        // Notification channel for new account discoveries (reactive, not polling)
        let (new_account_tx, mut new_account_rx) = mpsc::channel::<u32>(10);

        let abort_signal = {
            let manager = manager.clone();
            let output = output.clone();

            tokio::spawn(live_capture(
                manager,
                sniffer,
                MappedSender::new(output, WorkerEvent::Metric),
                recorded_rx,
                new_account_tx,
            ))
        };

        output
            .send(WorkerEvent::Ready(sender.clone()))
            .await
            .expect("Worker Stream was closed before ready state?");

        loop {
            tokio::select! {
                // New account discovered or reconnected (reactive notification from live_capture)
                uid = new_account_rx.select_next_some() => {
                    let current_exporter = manager.lock().await.get_account_exporter(uid);
                    
                    if let Some(exporter) = current_exporter {
                        // Check if we need to resubscribe (new account or reconnection with different exporter)
                        let (should_subscribe, is_reconnection) = if let Some((old_exporter, old_task)) = subscriptions.get(&uid) {
                            // Check if exporter instance changed (reconnection)
                            if !Arc::ptr_eq(old_exporter, &exporter) {
                                info!(uid, "detected reconnection - exporter changed, aborting old subscription");
                                old_task.abort();
                                (true, true)
                            } else {
                                // Same exporter, subscription still valid
                                (false, false)
                            }
                        } else {
                            // No existing subscription
                            info!(uid, "new account discovered");
                            (true, false)
                        };
                        
                        // Notify GUI about reconnection
                        if is_reconnection {
                            output.send(WorkerEvent::AccountReconnected { uid }).await.ok();
                        }
                        
                        if should_subscribe {
                            info!(uid, "subscribing to account events");
                            let (initial_event, mut rx) = exporter.lock().await.subscribe();
                            
                            // Send initial event if available
                            if let Some(event) = initial_event {
                                info!(uid, "sending initial event to GUI");
                                output.send(WorkerEvent::ExportEvent { uid, event }).await.ok();
                            } else {
                                info!(uid, "no initial event (exporter not initialized yet)");
                            }
                            
                            // Spawn a task to forward events from this subscription
                            let mut output_clone = output.clone();
                            let exporter_clone = exporter.clone();
                            let task = tokio::spawn(async move {
                                info!(uid, "event forwarding task started");
                                loop {
                                    match rx.recv().await {
                                        Ok(event) => {
                                            info!(uid, ?event, "forwarding event to GUI");
                                            if output_clone.send(WorkerEvent::ExportEvent { uid, event }).await.is_err() {
                                                warn!(uid, "output channel closed");
                                                break;
                                            }
                                        }
                                        Err(e) => {
                                            warn!(uid, ?e, "subscription closed or lagged");
                                            break;
                                        }
                                    }
                                }
                                info!(uid, "event forwarding task ended");
                            });
                            
                            subscriptions.insert(uid, (exporter_clone, task));
                        }
                    } else {
                        warn!(uid, "no exporter found for discovered account");
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
                                if let Some(exporter) = exporter_opt {
                                    exporter.lock().await.export()
                                } else {
                                    None
                                }
                            } else {
                                // Export first account
                                let accounts = manager.lock().await.get_all_accounts();
                                if let Some((_, exporter)) = accounts.first() {
                                    exporter.lock().await.export()
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
async fn live_capture(
    manager: Arc<Mutex<MultiAccountManager>>,
    mut sniffer: GameSniffer,
    mut metric_tx: MappedSender<WorkerEvent, SnifferMetric>,
    mut recorded_rx: mpsc::Receiver<capture::Packet>,
    mut new_account_tx: mpsc::Sender<u32>,
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
                                ConnectionPacket::HandshakeEstablished { conv_id } => {
                                    info!(conv_id, "detected connection established");
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
                            GamePacket::Commands { conv_id, result } => match result {
                                Ok(command) => {
                                    if command.command_id == PlayerLoginScRsp {
                                        info!(conv_id, "detected login start");
                                    }

                                    // Check if this is a UID discovery packet
                                    if command.command_id == PlayerGetTokenScRsp {
                                        if let Ok(token_rsp) = command.parse_proto::<PlayerGetTokenScRspProto>() {
                                            let uid = token_rsp.uid;
                                            let mut mgr = manager.lock().await;
                                            let is_new = mgr.register_uid(conv_id, uid);
                                            
                                            // Emit account discovered event
                                            metric_tx.sender.send(WorkerEvent::AccountDiscovered { 
                                                uid, 
                                                is_active: true 
                                            }).await.ok();
                                            
                                            // Always notify worker (handles both new accounts and reconnections)
                                            new_account_tx.send(uid).await.ok();
                                        }
                                    }

                                    // Route command to the correct exporter
                                    let exporter = {
                                        let mut mgr = manager.lock().await;
                                        mgr.get_or_create_exporter(conv_id)
                                    };
                                    
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
    }
}
