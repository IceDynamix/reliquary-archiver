//! Turns captured packets into exporter state.
//!
//! A [`CaptureSession`] is the single place where packets are decoded and routed to the
//! per-account exporters. It reports what happened as [`SessionEvent`]s and leaves the policy
//! (when to stop, what to show the user) to whoever drives it: the CLI or the GUI worker.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures::lock::Mutex;
use reliquary::network::command::command_id::{self, PlayerGetTokenScRsp, PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::command::proto::DoGachaCsReq::DoGachaCsReq as DoGachaCsReqProto;
use reliquary::network::command::proto::DoGachaScRsp::DoGachaScRsp as DoGachaScRspProto;
use reliquary::network::command::proto::GetAuthkeyScRsp::GetAuthkeyScRsp as GetAuthkeyScRspProto;
use reliquary::network::command::proto::GetGachaInfoScRsp::GetGachaInfoScRsp as GetGachaInfoScRspProto;
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp as PlayerGetTokenScRspProto;
use reliquary::network::command::{GameCommand, GameCommandError};
use reliquary::network::{ConnectionPacket, ConnectionPacketError, GamePacket, GameSniffer, NetworkError};
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::{Export, ExporterState, OptimizerExporter};
use tracing::{info, trace, warn};

use crate::accounts::MultiAccountManager;
use crate::capture::Packet;
use crate::merge::{MessageKey, SourceMerger};
use crate::stall::{STALL_THRESHOLD, StallChange, StallDetector};
use crate::warps::{BannerInfo, LiveRequest, WarpEvent};

/// The web app the game asks an authkey for when it opens the warp history
const WARP_HISTORY_APP: &str = "webview_gacha";

/// How often at most what changed is handed over to be kept, see [`CaptureSession::resume_from`]
const KEEP_EVERY: Duration = Duration::from_secs(2);

/// A connection's account, session key and captured state, kept to pick the connection up again
/// after a restart
#[derive(Debug, Clone)]
pub struct SavedSession {
    pub conv_id: u32,
    pub uid: u32,
    /// The seed of the session key (`PlayerGetTokenScRsp.secret_key_seed`)
    pub seed: u64,
    pub state: ExporterState,
}

/// Sessions kept from earlier runs and where to send what to keep
struct Resume {
    /// By conversation, until traffic of theirs decodes
    saved: HashMap<u32, SavedSession>,
    keep: Box<dyn Fn(SavedSession) + Send + Sync>,
    /// The [generation](OptimizerExporter::generation) each conversation's exporter was last
    /// kept at
    kept: HashMap<u32, u64>,
    last_kept: Instant,
}

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
    /// An account's connection was picked up from the last run (see
    /// [`CaptureSession::resume_from`]), right after its [`SessionEvent::AccountSeen`]. Its data
    /// is what was kept then: whatever changed while the archiver was closed is missing until
    /// it logs in again, which brings a plain [`SessionEvent::AccountSeen`].
    PickedUp {
        uid: u32,
    },
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
    latest_uid: Option<u32>,
    stall: StallDetector,
    /// The account each connection logged in as
    conv_uids: HashMap<u32, u32>,
    /// The warp each connection asked for last, until its result arrives: (banner, request counter)
    gacha_requests: HashMap<u32, (u32, u32)>,
    /// The seed of each connection's session key
    seeds: HashMap<u32, u64>,
    resume: Option<Resume>,
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
            latest_uid: None,
            stall: StallDetector::default(),
            conv_uids: HashMap::new(),
            gacha_requests: HashMap::new(),
            seeds: HashMap::new(),
            resume: None,
        }
    }

    /// Picks up the `saved` sessions when their connections turn out to be still going: when
    /// a command of theirs decrypts with the kept key, their exporters carry on from the kept
    /// state, as if the login had been captured. From then on, every session's account, key
    /// and state go to `keep` whenever they change, [`KEEP_EVERY`] at most.
    pub fn resume_from(&mut self, saved: Vec<SavedSession>, keep: impl Fn(SavedSession) + Send + Sync + 'static) {
        let saved: HashMap<u32, SavedSession> = saved.into_iter().map(|session| (session.conv_id, session)).collect();
        for sniffer in self.sniffers.values_mut() {
            for session in saved.values() {
                sniffer.set_session_key(session.conv_id, session.seed);
            }
        }
        self.resume = Some(Resume {
            saved,
            keep: Box::new(keep),
            kept: HashMap::new(),
            last_kept: Instant::now(),
        });
    }

    pub async fn handle_packet(&mut self, packet: Packet) -> Vec<SessionEvent> {
        let mut events = self.decode_packet(packet).await;

        // Anything that got this far was game traffic, whether or not it decoded
        if !events.is_empty() {
            let change = self.stall.traffic(Instant::now());
            push_stall_change(&mut events, change);
        }

        self.keep_changed().await;
        events
    }

    /// Hands the sessions whose exporter changed to be kept, if it's been long enough. Whatever
    /// changed them, a command or anything else, shows in the exporter's generation.
    async fn keep_changed(&mut self) {
        let Some(resume) = self.resume.as_mut() else { return };
        if resume.last_kept.elapsed() < KEEP_EVERY {
            return;
        }
        resume.last_kept = Instant::now();
        for (&conv_id, &uid) in &self.conv_uids {
            let Some(&seed) = self.seeds.get(&conv_id) else { continue };
            let Some(exporter) = self.manager.lock().await.get_conv_exporter(conv_id) else {
                continue;
            };
            let exporter = exporter.lock().await;
            let generation = exporter.generation();
            if resume.kept.get(&conv_id) == Some(&generation) {
                continue;
            }
            let state = exporter.state();
            drop(exporter);
            resume.kept.insert(conv_id, generation);
            (resume.keep)(SavedSession { conv_id, uid, seed, state });
        }
    }

    /// Restores the kept session of the connection, if it has one and it hasn't been seen yet
    async fn resume_connection(&mut self, conv_id: u32, command_id: u16, events: &mut Vec<SessionEvent>) {
        let Some(resume) = self.resume.as_mut() else { return };
        if self.conv_uids.contains_key(&conv_id) {
            return;
        }
        let Some(saved) = resume.saved.remove(&conv_id) else { return };
        if command_id == PlayerGetTokenScRsp {
            // A login of its own after all, which brings a new key and everything else
            return;
        }

        let exporter = self.manager.lock().await.get_or_create_exporter(conv_id);
        let mut restoring = exporter.lock().await;
        if !restoring.restore(saved.state) {
            return;
        }
        // What was restored is what's kept already
        resume.kept.insert(conv_id, restoring.generation());
        drop(restoring);
        info!(conv_id, uid = saved.uid, "picked up the connection from the last run");
        self.seeds.insert(conv_id, saved.seed);
        self.conv_uids.insert(conv_id, saved.uid);
        self.manager.lock().await.register_uid(conv_id, saved.uid);
        self.latest_uid = Some(saved.uid);
        events.push(SessionEvent::AccountSeen { uid: saved.uid });
        events.push(SessionEvent::PickedUp { uid: saved.uid });
        if exporter.lock().await.is_initialized() {
            events.push(SessionEvent::LoginFinished);
        }
    }

    /// Decodes the packet and routes its commands. Returns no events for packets that aren't game traffic.
    async fn decode_packet(&mut self, packet: Packet) -> Vec<SessionEvent> {
        let mut events = Vec::new();
        let source = packet.source_id;

        if self.poisoned_sources.as_ref().is_some_and(|sources| sources.contains(&source)) {
            // We already know that this source is poisoned, so we can skip it
            return events;
        }

        let received = {
            let saved = self.resume.iter().flat_map(|resume| resume.saved.values());
            let sniffer = self.sniffers.entry(source).or_insert_with(|| {
                let mut sniffer = GameSniffer::new();
                for session in saved {
                    sniffer.set_session_key(session.conv_id, session.seed);
                }
                sniffer
            });
            sniffer.receive_packet(packet.data)
        };
        let packets = match received {
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
        self.resume_connection(conv_id, command.command_id, events).await;

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
            self.seeds.insert(conv_id, token_rsp.secret_key_seed);
            self.conv_uids.insert(conv_id, uid);
            self.manager.lock().await.register_uid(conv_id, uid);
            self.latest_uid = Some(uid);
            events.push(SessionEvent::AccountSeen { uid });
        }

        if let Some(event) = self.warp_event(conv_id, &command) {
            events.push(SessionEvent::Warp(event));
        }

        exporter.lock().await.read_command(command);
    }

    /// What the command says about warps, if anything
    fn warp_event(&mut self, conv_id: u32, command: &GameCommand) -> Option<WarpEvent> {
        match command.command_id {
            command_id::DoGachaCsReq => {
                // The request carries the account's warp request counter, the result doesn't
                let request = command.parse_proto::<DoGachaCsReqProto>().ok()?;
                self.gacha_requests.insert(conv_id, (request.gacha_id, request.gacha_random));
                None
            }
            command_id::DoGachaScRsp => {
                let result = command.parse_proto::<DoGachaScRspProto>().ok()?;
                let request_no = self
                    .gacha_requests
                    .remove(&conv_id)
                    .filter(|(gacha_id, _)| *gacha_id == result.gacha_id)
                    .map(|(_, counter)| counter);
                if result.retcode != 0 || result.gacha_item_list.is_empty() {
                    return None;
                }
                let Some(&uid) = self.conv_uids.get(&conv_id) else {
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
                let uid = *self.conv_uids.get(&conv_id)?;
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
                    .map(|banner| {
                        let mut rate_up: Vec<u32> = banner.prize_item_list.iter().chain(&banner.item_detail_list).copied().collect();
                        rate_up.sort_unstable();
                        rate_up.dedup();
                        BannerInfo {
                            gacha_id: banner.gacha_id,
                            begin: banner.begin_time,
                            end: banner.end_time,
                            rate_up,
                        }
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

    /// Replays the captures in `RQA_CAPTURES` (4.7standard.pcapng, 4.7recordauthkey.pcapng) and
    /// checks the warp events they make
    #[tokio::test]
    async fn recorded_warps_and_authkeys_come_out_of_a_capture() {
        use futures::StreamExt;

        let Ok(dir) = std::env::var("RQA_CAPTURES") else {
            return;
        };
        let replay = |name: &str| crate::capture::Replay::for_path(std::path::Path::new(&dir).join(name)).unwrap();
        async fn warps(replay: crate::capture::Replay) -> Vec<WarpEvent> {
            let mut session = session();
            let mut packets = replay.open().unwrap();
            let mut found = Vec::new();
            while let Some(packet) = packets.next().await {
                for event in session.handle_packet(packet.unwrap()).await {
                    if let SessionEvent::Warp(event) = event {
                        found.push(event);
                    }
                }
            }
            found
        }

        let pulls: Vec<LiveRequest> = warps(replay("4.7standard.pcapng"))
            .await
            .into_iter()
            .filter_map(|event| match event {
                WarpEvent::Pulled(request) => Some(request),
                _ => None,
            })
            .collect();
        let summary: Vec<(u32, Option<u32>, Vec<u32>)> = pulls.iter().map(|p| (p.gacha_id, p.request_no, p.items.clone())).collect();
        assert_eq!(summary, [
            (2142, Some(1576), vec![20011]),
            (1001, Some(1577), vec![20000]),
            (1001, Some(1578), vec![20015]),
            (1001, Some(1579), vec![20002]),
            (1001, Some(1580), vec![20010]),
            (1001, Some(1581), vec![1106]),
        ]);
        assert!(pulls.iter().all(|p| p.uid == 602857786));

        let events = warps(replay("4.7recordauthkey.pcapng")).await;
        let keys: Vec<&WarpEvent> = events.iter().filter(|e| matches!(e, WarpEvent::Authkey { .. })).collect();
        // The survey authkey the game asks for at login is left out
        assert_eq!(keys.len(), 1);
        assert!(matches!(keys[0], WarpEvent::Authkey { uid: 602857786, authkey } if authkey.len() > 100));
        let banners = events.iter().find_map(|e| match e {
            WarpEvent::Banners { banners } => Some(banners),
            _ => None,
        });
        let banners = banners.expect("the warp screen's banners");
        println!(
            "banners: {:?}",
            banners.iter().map(|b| (b.gacha_id, &b.rate_up)).collect::<Vec<_>>()
        );
        assert!(banners.iter().any(|b| b.gacha_id == 2142));
    }

    /// Replays 4.7standard.pcapng from `RQA_CAPTURES` as a restart halfway through would see it:
    /// a first run until the login is done, then a second that only gets the rest, with what
    /// the first one kept. The second should end up where a run over all of it does.
    #[tokio::test]
    async fn a_restart_picks_the_connection_up_with_the_kept_session() {
        use futures::StreamExt;

        let Ok(dir) = std::env::var("RQA_CAPTURES") else {
            return;
        };
        let replay = crate::capture::Replay::for_path(std::path::Path::new(&dir).join("4.7standard.pcapng")).unwrap();
        let packets: Vec<Packet> = replay.open().unwrap().map(|packet| packet.unwrap()).collect().await;

        let pulls = |events: &[SessionEvent]| {
            events
                .iter()
                .filter_map(|event| match event {
                    SessionEvent::Warp(WarpEvent::Pulled(request)) => Some((request.request_no, request.items.clone())),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        let export = async |session: &CaptureSession| serde_json::to_value(session.export().await.unwrap()).unwrap();

        // All of it in one run
        let mut whole = session();
        let mut whole_events = Vec::new();
        for packet in &packets {
            whole_events.extend(whole.handle_packet(packet.clone()).await);
        }

        // The first run stops a little after the login, before the warps
        let mut first = session();
        let mut stop = None;
        for (i, packet) in packets.iter().enumerate() {
            let events = first.handle_packet(packet.clone()).await;
            if stop.is_none() && events.contains(&SessionEvent::LoginFinished) {
                stop = Some(i + 200);
            }
            if stop == Some(i) {
                break;
            }
        }
        let stop = stop.expect("the capture has a login");
        let (&conv_id, &uid) = first.conv_uids.iter().next().unwrap();
        let state = first.manager.lock().await.get_conv_exporter(conv_id).unwrap().lock().await.state();
        let saved = SavedSession {
            conv_id,
            uid,
            seed: first.seeds[&conv_id],
            state: serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap(),
        };

        // The second gets the rest
        let mut second = session();
        second.resume_from(vec![saved], |_| {});
        let mut second_events = Vec::new();
        for packet in &packets[stop + 1..] {
            second_events.extend(second.handle_packet(packet.clone()).await);
        }

        assert!(second_events.contains(&SessionEvent::AccountSeen { uid }));
        assert!(second_events.contains(&SessionEvent::PickedUp { uid }));
        assert!(second.is_initialized().await);
        assert!(!pulls(&second_events).is_empty(), "the warps after the restart are seen");
        assert!(pulls(&whole_events).ends_with(&pulls(&second_events)));
        assert_eq!(export(&second).await, export(&whole).await);
    }

    #[tokio::test]
    async fn the_same_disconnect_from_several_sources_is_reported_once() {
        let mut session = session();

        let events = from_each_source(&mut session, &[1, 2, 3], &disconnected()).await;
        assert_eq!(count(&events, SessionEvent::Disconnected), 1);
    }
}
