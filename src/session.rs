//! Turns captured packets into exporter state.
//!
//! A [`CaptureSession`] is the single place where packets are decoded and routed to the
//! per-account exporters. It reports what happened as [`SessionEvent`]s and leaves the policy
//! (when to stop, what to show the user) to whoever drives it: the CLI or the GUI worker.

use std::collections::HashSet;
use std::sync::Arc;

use futures::lock::Mutex;
use reliquary::network::command::command_id::{PlayerGetTokenScRsp, PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp as PlayerGetTokenScRspProto;
use reliquary::network::command::{GameCommand, GameCommandError};
use reliquary::network::{ConnectionPacket, ConnectionPacketError, GamePacket, GameSniffer, KcpError, NetworkError};
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::{Export, OptimizerExporter};
use tracing::{info, trace, warn};

use crate::accounts::MultiAccountManager;
use crate::capture::Packet;

/// Something that happened while processing a packet
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SessionEvent {
    ConnectionEstablished,
    Disconnected,
    /// A packet was decoded into this many game packets
    PacketsDecoded(usize),
    /// An account logged in. Reported on every login, also for accounts that have been seen before
    AccountSeen {
        uid: u32,
    },
    /// The game finished sending its login data
    LoginFinished,
    /// The traffic can't be decrypted, which means the game version is not supported
    DecryptionKeyMissing,
    /// A packet or command could not be decoded, capturing can continue
    DecodeError,
    /// The sniffer can't make sense of the traffic anymore
    Fatal,
}

pub struct CaptureSession {
    sniffer: GameSniffer,
    manager: Arc<Mutex<MultiAccountManager>>,
    /// Sources that sent something that isn't game traffic and are ignored from then on
    poisoned_sources: Option<HashSet<u64>>,
    latest_uid: Option<u32>,
}

impl CaptureSession {
    /// `poison_sources` makes the session ignore a source for good once it delivers a packet
    /// that can't be parsed, which is what short-lived captures over all network interfaces want.
    pub fn new(manager: Arc<Mutex<MultiAccountManager>>, poison_sources: bool) -> Self {
        Self {
            sniffer: GameSniffer::new(),
            manager,
            poisoned_sources: poison_sources.then(HashSet::new),
            latest_uid: None,
        }
    }

    pub async fn handle_packet(&mut self, packet: Packet) -> Vec<SessionEvent> {
        let mut events = Vec::new();

        if self
            .poisoned_sources
            .as_ref()
            .is_some_and(|sources| sources.contains(&packet.source_id))
        {
            // We already know that this source is poisoned, so we can skip it
            return events;
        }

        let packets = match self.sniffer.receive_packet(packet.data) {
            Ok(packets) => packets,
            Err(NetworkError::ConnectionPacket(ConnectionPacketError::IncorrectPorts)) => {
                // Not game traffic, which is expected from sources that don't filter by port
                trace!("ignoring packet with unrelated ports");
                return events;
            }
            Err(e) => {
                warn!(%e);
                events.push(self.network_error_event(e, packet.source_id));
                return events;
            }
        };

        events.push(SessionEvent::PacketsDecoded(packets.len()));

        for packet in packets {
            match packet {
                GamePacket::Connection(ConnectionPacket::HandshakeEstablished { conv_id }) => {
                    info!(conv_id, "detected connection established");

                    if cfg!(all(feature = "pcap", windows)) {
                        info!(
                            "If the program gets stuck at this point for longer than 10 seconds, please try the pktmon release from https://github.com/IceDynamix/reliquary-archiver/releases/latest"
                        );
                    }

                    events.push(SessionEvent::ConnectionEstablished);
                }
                GamePacket::Connection(ConnectionPacket::Disconnected) => {
                    info!("detected connection disconnected");
                    events.push(SessionEvent::Disconnected);
                }
                GamePacket::Connection(_) => {}
                GamePacket::Commands {
                    conv_id,
                    result: Ok(command),
                } => {
                    self.handle_command(conv_id, command, &mut events).await;
                }
                GamePacket::Commands { conv_id, result: Err(e) } => {
                    warn!(conv_id, %e);
                    events.push(command_error_event(&e));
                }
            }
        }

        events
    }

    async fn handle_command(&mut self, conv_id: u32, command: GameCommand, events: &mut Vec<SessionEvent>) {
        if command.command_id == PlayerLoginScRsp {
            info!(conv_id, "detected login start");
        }

        if command.command_id == PlayerLoginFinishScRsp {
            events.push(SessionEvent::LoginFinished);
        }

        // Route command to the correct exporter
        let exporter = self.manager.lock().await.get_or_create_exporter(conv_id);

        // Check if this is a UID discovery packet and register it if so
        if command.command_id == PlayerGetTokenScRsp
            && let Ok(token_rsp) = command.parse_proto::<PlayerGetTokenScRspProto>()
        {
            let uid = token_rsp.uid;
            self.manager.lock().await.register_uid(conv_id, uid);
            self.latest_uid = Some(uid);
            events.push(SessionEvent::AccountSeen { uid });
        }

        exporter.lock().await.read_command(command);
    }

    fn network_error_event(&mut self, error: NetworkError, source_id: u64) -> SessionEvent {
        match error {
            NetworkError::ConnectionPacket(e) => {
                if let ConnectionPacketError::TransportLayerNotPresent = e {
                    warn!("This error tends to happen when using VPNs or similar programs. Please disable them and try again.");
                }

                // Connection errors are not fatal as all network interfaces are funneled through the same stream
                // Just mark this source as poisoned and continue listening on other sources
                if let Some(sources) = self.poisoned_sources.as_mut() {
                    sources.insert(source_id);
                }

                SessionEvent::DecodeError
            }
            // reliquary should assign conversation ids correctly, if it doesn't then something went wrong
            NetworkError::Kcp(KcpError::PacketDoesNotBelongToConversation { .. } | KcpError::InnerKcpError(_)) => SessionEvent::Fatal,
            // structural parse errors
            NetworkError::Kcp(_) => SessionEvent::DecodeError,
            NetworkError::GameCommand(e) => command_error_event(&e),
        }
    }

    /// The exporter of the account that logged in most recently
    pub async fn latest_exporter(&self) -> Option<Arc<Mutex<OptimizerExporter>>> {
        self.manager.lock().await.get_account_exporter(self.latest_uid?)
    }

    /// Whether everything needed for an export of the latest account has been captured
    pub async fn is_initialized(&self) -> bool {
        match self.latest_exporter().await {
            Some(exporter) => exporter.lock().await.is_initialized(),
            None => false,
        }
    }

    /// Export whatever has been captured for the latest account
    pub async fn export(&self) -> Option<Export> {
        self.latest_exporter().await?.lock().await.export()
    }
}

fn command_error_event(error: &GameCommandError) -> SessionEvent {
    match error {
        // version is not supported, there's no point in capturing
        GameCommandError::DecryptionKeyMissing => SessionEvent::DecryptionKeyMissing,
        // HeaderTooShort/CommandTooShort are structural parse errors.
        // VersionMismatch means the client packet was likely misordered from the server packet,
        // it will be reprocessed after we receive the new session key
        _ => SessionEvent::DecodeError,
    }
}

/// Reads a recorded capture from start to end and exports the account found in it
#[cfg(feature = "gui")]
pub async fn export_from_file(spec: crate::capture::SourceSpec) -> Option<Export> {
    use futures::StreamExt;

    let mut packets = match crate::capture::open_source(&spec) {
        Ok(packets) => packets,
        Err(e) => {
            tracing::error!(%e, "could not read capture file");
            return None;
        }
    };

    let mut session = CaptureSession::new(Arc::new(Mutex::new(MultiAccountManager::new())), false);

    while let Some(packet) = packets.next().await {
        match packet {
            Ok(packet) => {
                session.handle_packet(packet).await;
            }
            Err(e) => warn!(%e),
        }

        if session.is_initialized().await {
            info!("finished capturing");
            break;
        }
    }

    session.export().await
}
