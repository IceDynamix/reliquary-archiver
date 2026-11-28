//! Turns captured packets into exporter state.
//!
//! A [`CaptureSession`] is the single place where packets are decoded and routed to the
//! per-account exporters. It reports what happened as [`SessionEvent`]s and leaves the policy
//! (when to stop, what to show the user) to whoever drives it: the CLI or the GUI worker.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use futures::lock::Mutex;
use reliquary::network::command::command_id::{self, PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::command::proto::DoGachaScRsp::DoGachaScRsp as DoGachaScRspProto;
use reliquary::network::command::proto::GetAuthkeyScRsp::GetAuthkeyScRsp as GetAuthkeyScRspProto;
use reliquary::network::command::proto::GetGachaInfoScRsp::GetGachaInfoScRsp as GetGachaInfoScRspProto;
use reliquary::network::command::{GameCommand, GameCommandError};
use reliquary::network::{ConnectionPacket, ConnectionPacketError, GamePacket, GameSniffer, NetworkError};
use reliquary_archiver::export::fribbels::Export;
use reliquary_archiver::export::{Exporter, banner_rate_up};
use tracing::{info, trace, warn};

use crate::accounts::MultiAccountManager;
use crate::capture::Packet;
use crate::merge::{MessageKey, SourceMerger};
use crate::stall::{STALL_THRESHOLD, StallChange, StallDetector};
use crate::warps::{BannerInfo, LiveRequest, WarpEvent};

/// The web app the game asks an authkey for when it opens the warp history
const WARP_HISTORY_APP: &str = "webview_gacha";

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
    /// Game traffic keeps arriving but nothing decodes from it, so the capture missed data and
    /// the rest of this connection can't be read. Logging in again starts a fresh connection.
    Stalled,
    /// Commands are being decoded again after [`SessionEvent::Stalled`]
    Recovered,
    /// Something to keep in the warp history
    Warp(WarpEvent),
}

/// Decodes packets into commands for the exporters.
///
/// Every capture source gets its own sniffer. Sources often see the same traffic (several network
/// interfaces, or pktmon reporting a packet once per component it passes), and a source that
/// misses or mangles a packet would otherwise break decoding for all of them. What the sniffers
/// decode goes through a [`SourceMerger`], so each message reaches the exporters once, from
/// whichever source produced it first.
pub struct CaptureSession {
    sniffers: HashMap<u64, GameSniffer>,
    merger: SourceMerger,
    manager: Arc<Mutex<MultiAccountManager>>,
    /// Sources that sent something that isn't game traffic and are ignored from then on
    poisoned_sources: Option<HashSet<u64>>,
    stall: StallDetector,
}

impl CaptureSession {
    /// `poison_sources` makes the session ignore a source for good once it delivers a packet
    /// that can't be parsed, which is what short-lived captures over all network interfaces want.
    pub fn new(manager: Arc<Mutex<MultiAccountManager>>, poison_sources: bool) -> Self {
        Self {
            sniffers: HashMap::new(),
            merger: SourceMerger::new(),
            manager,
            poisoned_sources: poison_sources.then(HashSet::new),
            stall: StallDetector::default(),
        }
    }

    pub async fn handle_packet(&mut self, packet: Packet) -> Vec<SessionEvent> {
        let mut events = self.decode_packet(packet).await;

        // Anything that got this far was game traffic, whether or not it decoded
        if !events.is_empty() {
            let change = self.stall.traffic(Instant::now());
            push_stall_change(&mut events, change);
        }

        events
    }

    /// Decodes the packet and routes its commands. Returns no events for packets that aren't game traffic.
    async fn decode_packet(&mut self, packet: Packet) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        let source = packet.source_id;

        if self.poisoned_sources.as_ref().is_some_and(|sources| sources.contains(&source)) {
            // We already know that this source is poisoned, so we can skip it
            return events;
        }

        let sniffer = self.sniffers.entry(source).or_default();
        let packets = match sniffer.receive_packet(packet.data) {
            Ok(packets) => packets,
            Err(NetworkError::ConnectionPacket(ConnectionPacketError::IncorrectPorts)) => {
                // Not game traffic, which is expected from sources that don't filter by port
                trace!("ignoring packet with unrelated ports");
                return events;
            }
            Err(e) => {
                warn!(source, %e);
                events.push(self.network_error_event(e, source));
                return events;
            }
        };

        let now = Instant::now();
        let mut delivered = 0;
        let mut delivered_events = Vec::new();

        for packet in packets {
            let key = match &packet {
                GamePacket::Connection(ConnectionPacket::HandshakeEstablished { conv_id }) => {
                    Some(MessageKey::HandshakeEstablished { conv_id: *conv_id })
                }
                GamePacket::Connection(ConnectionPacket::Disconnected) => Some(MessageKey::Disconnected),
                GamePacket::Commands {
                    conv_id,
                    result: Ok(command),
                } => Some(MessageKey::command(*conv_id, command)),
                // Errors are this source's own, report them as they come
                GamePacket::Commands { result: Err(_), .. } | GamePacket::Connection(_) => None,
            };

            if let Some(key) = key {
                if !self.merger.accept(source, key, now) {
                    // Another source got here first
                    continue;
                }
                delivered += 1;
            }

            self.handle_game_packet(packet, &mut delivered_events).await;
        }

        events.push(SessionEvent::PacketsDecoded(delivered));
        events.append(&mut delivered_events);
        events
    }

    async fn handle_game_packet(&mut self, packet: GamePacket, events: &mut Vec<SessionEvent>) {
        match packet {
            GamePacket::Connection(ConnectionPacket::HandshakeEstablished { conv_id }) => {
                info!(conv_id, "detected connection established");

                if cfg!(all(feature = "pcap", windows)) {
                    info!(
                        "If the program gets stuck at this point for longer than 10 seconds, please try the pktmon release from https://github.com/IceDynamix/reliquary-archiver/releases/latest"
                    );
                }

                events.push(SessionEvent::ConnectionEstablished);

                let change = self.stall.connection_established(Instant::now());
                push_stall_change(events, change);
            }
            GamePacket::Connection(ConnectionPacket::Disconnected) => {
                info!("detected connection disconnected");
                events.push(SessionEvent::Disconnected);

                let change = self.stall.disconnected();
                push_stall_change(events, change);
            }
            GamePacket::Connection(_) => {}
            GamePacket::Commands {
                conv_id,
                result: Ok(command),
            } => {
                // Any source decoding counts as progress, and only the first copy gets this far
                let change = self.stall.command_decoded(Instant::now());
                push_stall_change(events, change);

                self.handle_command(conv_id, command, events).await;
            }
            GamePacket::Commands { conv_id, result: Err(e) } => {
                warn!(conv_id, %e);
                events.push(command_error_event(&e));
            }
        }
    }

    async fn handle_command(&mut self, conv_id: u32, command: GameCommand, events: &mut Vec<SessionEvent>) {
        if command.command_id == PlayerLoginScRsp {
            info!(conv_id, "detected login start");
        }

        if command.command_id == PlayerLoginFinishScRsp {
            events.push(SessionEvent::LoginFinished);
        }

        let manager = self.manager.clone();
        let mut manager = manager.lock().await;

        if let Some(event) = self.warp_event(manager.uid_of(conv_id), &command) {
            events.push(SessionEvent::Warp(event));
        }

        // Route command to the correct exporter
        if let Some(uid) = manager.read_command(conv_id, command) {
            events.push(SessionEvent::AccountSeen { uid });
        }
    }

    /// What the command says about warps, if anything. `uid` is the account the connection logged in as.
    fn warp_event(&self, uid: Option<u32>, command: &GameCommand) -> Option<WarpEvent> {
        match command.command_id {
            command_id::DoGachaScRsp => {
                let result = command.parse_proto::<DoGachaScRspProto>().ok()?;
                // The result carries the counter the account's next warp request will send, one
                // more than this request's
                let request_no = result.next_gacha_random.checked_sub(1);
                if result.retcode != 0 || result.gacha_item_list.is_empty() {
                    return None;
                }
                let Some(uid) = uid else {
                    warn!("a warp was made before the account was known, it can't be kept");
                    return None;
                };
                Some(WarpEvent::Pulled(LiveRequest {
                    uid,
                    gacha_id: result.gacha_id,
                    request_no,
                    items: result.gacha_item_list.iter().map(|item| item.gacha_item.item_id).collect(),
                    captured_at: jiff::Timestamp::now().as_millisecond(),
                }))
            }
            command_id::GetAuthkeyScRsp => {
                let response = command.parse_proto::<GetAuthkeyScRspProto>().ok()?;
                if response.retcode != 0 || response.auth_appid != WARP_HISTORY_APP || response.authkey.is_empty() {
                    return None;
                }
                let uid = uid?;
                Some(WarpEvent::Authkey {
                    uid,
                    authkey: response.authkey,
                })
            }
            command_id::GetGachaInfoScRsp => {
                let info = command.parse_proto::<GetGachaInfoScRspProto>().ok()?;
                let banners = info
                    .gacha_info_list
                    .iter()
                    .map(|banner| BannerInfo {
                        gacha_id: banner.gacha_id,
                        begin: banner.begin_time,
                        end: banner.end_time,
                        rate_up: banner_rate_up(banner),
                    })
                    .collect();
                Some(WarpEvent::Banners { banners })
            }
            _ => None,
        }
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
                    self.sniffers.remove(&source_id);
                }

                SessionEvent::DecodeError
            }
            // Each source decodes on its own, so a KCP error only affects the source it came from
            // and the others carry on
            NetworkError::Kcp(_) => SessionEvent::DecodeError,
            NetworkError::GameCommand(e) => command_error_event(&e),
        }
    }

    /// Whether everything needed for an export of the latest account has been captured
    pub async fn is_initialized(&self) -> bool {
        self.manager
            .lock()
            .await
            .latest_exporter()
            .is_some_and(|exporter| exporter.is_initialized())
    }

    /// Export whatever has been captured for the latest account
    pub async fn export(&self) -> Option<Export> {
        self.manager.lock().await.latest_exporter()?.export()
    }
}

fn push_stall_change(events: &mut Vec<SessionEvent>, change: Option<StallChange>) {
    match change {
        Some(StallChange::Stalled) => {
            warn!(
                "game traffic is still arriving, but nothing could be decoded from it for {} seconds. The capture missed some data, \
                 so the rest of this login can't be read. Log out to the title screen and log back in to capture again.",
                STALL_THRESHOLD.as_secs()
            );
            events.push(SessionEvent::Stalled);
        }
        Some(StallChange::Recovered) => {
            info!("capture is decoding game data again");
            events.push(SessionEvent::Recovered);
        }
        None => {}
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
pub async fn export_from_file(replay: crate::capture::Replay) -> Option<Export> {
    use futures::StreamExt;

    let mut packets = match replay.open() {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An Ethernet/IPv4/UDP frame from the game server carrying `payload`
    fn server_frame(payload: &[u8]) -> Vec<u8> {
        let udp_len = 8 + payload.len() as u16;
        let ip_len = 20 + udp_len;

        let mut frame = Vec::new();
        frame.extend_from_slice(&[0; 12]); // MAC addresses
        frame.extend_from_slice(&0x0800u16.to_be_bytes()); // IPv4
        frame.extend_from_slice(&[0x45, 0]);
        frame.extend_from_slice(&ip_len.to_be_bytes());
        frame.extend_from_slice(&[0, 0, 0, 0, 64, 17, 0, 0]); // id, flags, ttl, UDP, checksum
        frame.extend_from_slice(&[10, 0, 0, 1, 10, 0, 0, 2]); // addresses
        frame.extend_from_slice(&23301u16.to_be_bytes());
        frame.extend_from_slice(&50000u16.to_be_bytes());
        frame.extend_from_slice(&udp_len.to_be_bytes());
        frame.extend_from_slice(&[0, 0]); // checksum
        frame.extend_from_slice(payload);
        frame
    }

    fn handshake_established(conv_id: u32) -> Vec<u8> {
        let mut payload = [0u8; 20];
        payload[..4].copy_from_slice(&0x145u32.to_be_bytes());
        payload[4..8].copy_from_slice(&conv_id.to_be_bytes());
        server_frame(&payload)
    }

    fn disconnected() -> Vec<u8> {
        let mut payload = [0u8; 20];
        payload[..4].copy_from_slice(&404u32.to_be_bytes());
        server_frame(&payload)
    }

    fn session() -> CaptureSession {
        CaptureSession::new(Arc::new(Mutex::new(MultiAccountManager::new())), false)
    }

    /// Feeds `data` to the session once per source and collects every event
    async fn from_each_source(session: &mut CaptureSession, sources: &[u64], data: &[u8]) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        for &source_id in sources {
            let packet = Packet {
                source_id,
                data: data.to_vec(),
            };
            events.extend(session.handle_packet(packet).await);
        }
        events
    }

    fn count(events: &[SessionEvent], event: SessionEvent) -> usize {
        events.iter().filter(|e| **e == event).count()
    }

    #[tokio::test]
    async fn the_same_handshake_from_several_sources_is_reported_once() {
        let mut session = session();

        let events = from_each_source(&mut session, &[1, 2, 3], &handshake_established(7)).await;
        assert_eq!(count(&events, SessionEvent::ConnectionEstablished), 1);
        assert_eq!(count(&events, SessionEvent::PacketsDecoded(1)), 1);
        assert_eq!(count(&events, SessionEvent::PacketsDecoded(0)), 2);

        // Each source still decodes on its own
        assert_eq!(session.sniffers.len(), 3);
    }

    #[tokio::test]
    async fn a_new_connection_is_reported_again() {
        let mut session = session();

        from_each_source(&mut session, &[1, 2], &handshake_established(7)).await;
        let events = from_each_source(&mut session, &[1, 2], &handshake_established(8)).await;
        assert_eq!(count(&events, SessionEvent::ConnectionEstablished), 1);
    }

    /// A command carrying `proto`, as the sniffer hands it on
    fn command(command_id: u16, proto: &impl protobuf::Message) -> GameCommand {
        let proto_data = proto.write_to_bytes().unwrap();
        GameCommand {
            command_id,
            header_len: 0,
            data_len: proto_data.len() as u32,
            proto_header: Vec::new(),
            proto_data,
        }
    }

    #[test]
    fn a_warp_is_reported_with_its_request_counter() {
        use reliquary::network::command::proto::GachaItem::GachaItem;
        use reliquary::network::command::proto::Item::Item;

        let item = |item_id: u32| GachaItem {
            gacha_item: Some(Item {
                item_id,
                ..Default::default()
            })
            .into(),
            ..Default::default()
        };
        let pull = command(command_id::DoGachaScRsp, &DoGachaScRspProto {
            gacha_id: 2142,
            next_gacha_random: 1577,
            gacha_item_list: vec![item(20011), item(1106)],
            ..Default::default()
        });

        let event = session().warp_event(Some(602857786), &pull);
        let Some(WarpEvent::Pulled(request)) = event else {
            panic!("expected a pull, got {event:?}");
        };
        assert_eq!((request.uid, request.gacha_id, request.request_no), (602857786, 2142, Some(1576)));
        assert_eq!(request.items, [20011, 1106]);

        // A pull before the account is known can't be kept
        assert!(session().warp_event(None, &pull).is_none());
    }

    #[test]
    fn only_the_warp_history_authkey_is_reported() {
        let authkey = |auth_appid: &str| {
            command(command_id::GetAuthkeyScRsp, &GetAuthkeyScRspProto {
                auth_appid: auth_appid.into(),
                authkey: "key".into(),
                ..Default::default()
            })
        };
        let session = session();

        assert!(matches!(
            session.warp_event(Some(7), &authkey(WARP_HISTORY_APP)),
            Some(WarpEvent::Authkey { uid: 7, authkey }) if authkey == "key"
        ));
        // The game asks for one for its surveys at login too
        assert!(session.warp_event(Some(7), &authkey("webview_survey")).is_none());
    }

    #[test]
    fn banners_are_reported_with_their_rate_ups() {
        use reliquary::network::command::proto::GachaInfo::GachaInfo;

        let info = command(command_id::GetGachaInfoScRsp, &GetGachaInfoScRspProto {
            gacha_info_list: vec![GachaInfo {
                gacha_id: 2142,
                begin_time: 1794349800,
                end_time: 1797969600,
                prize_item_list: vec![1511, 1009, 1110, 1207],
                ..Default::default()
            }],
            ..Default::default()
        });

        let Some(WarpEvent::Banners { banners }) = session().warp_event(None, &info) else {
            panic!("expected the banners");
        };
        assert_eq!(banners.len(), 1);
        assert_eq!(banners[0].gacha_id, 2142);
        assert_eq!(banners[0].rate_up, [1009, 1110, 1207, 1511]);
    }

    #[tokio::test]
    async fn the_same_disconnect_from_several_sources_is_reported_once() {
        let mut session = session();

        let events = from_each_source(&mut session, &[1, 2, 3], &disconnected()).await;
        assert_eq!(count(&events, SessionEvent::Disconnected), 1);
    }
}
