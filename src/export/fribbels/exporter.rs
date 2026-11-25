use std::collections::{BTreeMap, HashMap};
use std::ops::{Deref, DerefMut};

use reliquary::network::command::{GameCommand, command_id};
#[cfg(feature = "stream")]
use tokio::sync::broadcast;
use tracing::{debug, info, instrument, trace, warn};

use crate::export::Exporter;
use crate::export::database::{Database, get_database};
use crate::export::fribbels::models::*;

pub struct OptimizerExporter {
    pub(super) database: &'static Database,
    /// Everything captured, which is also what's kept between runs
    pub(super) data: Tracked<ExporterData>,

    #[cfg(feature = "stream")]
    pub(super) event_channel: broadcast::Sender<OptimizerEvent>,
}

/// A value that counts the times it was borrowed to be written to, see
/// [`OptimizerExporter::generation`]. Reading through it leaves the count alone.
pub(super) struct Tracked<T> {
    value: T,
    generation: u64,
}

impl<T> Tracked<T> {
    fn new(value: T) -> Self {
        Self { value, generation: 0 }
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &T {
        &self.value
    }
}

impl<T> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut T {
        self.generation += 1;
        &mut self.value
    }
}

impl Default for OptimizerExporter {
    fn default() -> Self {
        Self::new()
    }
}

impl OptimizerExporter {
    pub fn new() -> OptimizerExporter {
        OptimizerExporter {
            database: get_database(),
            data: Tracked::new(ExporterData::default()),

            #[cfg(feature = "stream")]
            event_channel: broadcast::channel(16).0,
        }
    }

    /// Everything captured, to keep between runs and [`restore`](Self::restore) later
    pub fn state(&self) -> ExporterState {
        ExporterState {
            version: ExporterState::VERSION,
            data: (*self.data).clone(),
        }
    }

    /// Picks up from a [`state`](Self::state) kept earlier, as if its commands had been read
    /// again. Returns false, leaving the exporter as it was, for a state of another version.
    ///
    /// The exporter is changed in place, so whoever is subscribed to its events stays subscribed.
    pub fn restore(&mut self, state: ExporterState) -> bool {
        if state.version != ExporterState::VERSION {
            warn!(
                version = state.version,
                "kept exporter state is of another version, not restoring it"
            );
            return false;
        }
        *self.data = state.data;

        #[cfg(feature = "stream")]
        if self.data.initialized {
            self.emit_initial_scan();
        }
        true
    }

    /// Goes up whenever what the exporter captured may have changed, so whatever keeps its
    /// [`state`](Self::state) can tell whether it needs to again by comparing with the
    /// generation it last kept. It can go up without anything having changed, never the
    /// other way round.
    pub fn generation(&self) -> u64 {
        self.data.generation
    }

    /// The uid of the account, once it is known
    pub fn uid(&self) -> Option<u32> {
        self.data.uid
    }

    /// Which trailblazer the account plays, once it is known
    pub fn trailblazer(&self) -> Option<&'static str> {
        self.data.trailblazer
    }

    pub fn gacha_funds(&self) -> GachaFunds {
        self.data.gacha
    }

    /// The gacha banners by id
    pub fn banners(&self) -> &HashMap<u32, BannerInfo> {
        &self.data.banners
    }

    /// Materials by id
    pub fn materials(&self) -> &BTreeMap<u32, Material> {
        &self.data.materials
    }

    /// Light cones by unique id
    pub fn light_cones(&self) -> &BTreeMap<u32, LightCone> {
        &self.data.light_cones
    }

    /// Relics by unique id
    pub fn relics(&self) -> &BTreeMap<u32, Relic> {
        &self.data.relics
    }

    /// Characters by id.
    ///
    /// A multipath character's level and ascension come from its base avatar. Until that has been
    /// captured, they are 0, like in the export.
    pub fn characters(&self) -> &BTreeMap<u32, Character> {
        &self.data.characters
    }

    #[allow(unused_variables)]
    fn emit_event(&self, event: OptimizerEvent) {
        if self.data.initialized {
            // Send only fails if there are no active receivers. We don't care if this is the case.
            #[cfg(feature = "stream")]
            self.event_channel.send(event).ok();
        } else {
            // Don't start sending real-time updates until we've completed initialization.
        }
    }

    fn reset(&mut self) {
        self.data.initialized = false;
        self.data.uid = None;
        self.data.trailblazer = None;
        self.data.banners.clear();
        self.data.gacha = GachaFunds::default();
        self.data.materials.clear();
        self.data.light_cones.clear();
        self.data.relics.clear();
        self.data.characters.clear();
        self.data.multipath_base_avatars.clear();
        self.data.unresolved_multipath_characters.clear();
    }

    fn is_finishable(&self) -> bool {
        self.data.trailblazer.is_some()
            && self.data.uid.is_some()
            && !self.data.relics.is_empty()
            // at least one character has its level and ascension
            && self.data.characters.len() > self.data.unresolved_multipath_characters.len()
            && !self.data.light_cones.is_empty()
    }

    fn is_empty(&self) -> bool {
        self.data.trailblazer.is_none()
            && self.data.uid.is_none()
            && self.data.relics.is_empty()
            && self.data.characters.is_empty()
            && self.data.light_cones.is_empty()
    }

    #[cfg(feature = "stream")]
    fn emit_initial_scan(&self) {
        let export = self.export().expect("initial scan failed");
        self.emit_event(OptimizerEvent::InitialScan(export));
    }
}

/// Parses the payload of a command, logging if it doesn't have the expected shape
fn parse<T: protobuf::Message>(command: &GameCommand, description: &str) -> Option<T> {
    command
        .parse_proto()
        .inspect_err(|error| warn!(%error, "could not parse {description} command"))
        .ok()
}

impl Exporter for OptimizerExporter {
    type Export = Export;

    #[cfg(feature = "stream")]
    type LiveEvent = OptimizerEvent;

    fn read_command(&mut self, command: GameCommand) {
        match command.command_id {
            command_id::PlayerGetTokenScRsp => {
                info!("detected new login attempt, resetting state");
                self.reset();

                debug!("detected uid");
                if let Some(cmd) = parse(&command, "token") {
                    self.handle_token(cmd);
                }
            }
            command_id::PlayerLoginScRsp => {
                debug!("detected login info packet");
                if let Some(cmd) = parse(&command, "login info data") {
                    self.handle_login(cmd);
                }
            }
            command_id::GetBagScRsp => {
                debug!("detected inventory packet");
                if let Some(cmd) = parse(&command, "inventory data") {
                    self.handle_inventory(cmd);
                }
            }
            command_id::GetAvatarDataScRsp => {
                debug!("detected character packet");
                if let Some(cmd) = parse(&command, "character data") {
                    self.handle_characters(cmd);
                }
            }
            command_id::SetAvatarEnhancedIdScRsp => {
                debug!("detected set avatar enhanced packet");
                if let Some(event) = parse(&command, "set avatar enhanced data").and_then(|cmd| self.handle_set_avatar_enhanced(cmd)) {
                    self.emit_event(event);
                }
            }
            command_id::GetGachaInfoScRsp => {
                debug!("detected gacha info packet");
                if let Some(cmd) = parse(&command, "gacha info data") {
                    self.handle_gacha_info(cmd);
                }
            }
            command_id::DoGachaScRsp => {
                debug!("detected gacha packet");
                if let Some(event) = parse(&command, "gacha data").and_then(|cmd| self.handle_gacha(cmd)) {
                    self.emit_event(event);
                }
            }
            command_id::PlayerSyncScNotify => {
                debug!("detected player sync packet");
                if let Some(cmd) = parse(&command, "player sync data") {
                    for event in self.handle_player_sync(cmd) {
                        self.emit_event(event);
                    }
                }
            }
            command_id::GetRelicBoxDataScRsp => {
                debug!("detected get relic recon data packet");
                if let Some(cmd) = parse(&command, "relic recon state") {
                    for event in self.handle_relic_box_data(cmd) {
                        self.emit_event(event);
                    }
                }
            }
            command_id::OpenRelicBoxScRsp => {
                debug!("detected open relic recon box packet");
                if let Some(event) = parse(&command, "relic recon drop").and_then(|cmd| self.handle_relic_box_open(cmd)) {
                    self.emit_event(event);
                }
            }
            command_id::ConfirmRelicBoxScRsp => {
                debug!("detected confirm relic recon choice packet");
                if let Some(event) = parse(&command, "relic recon confirmation").and_then(|cmd| self.handle_relic_box_confirm(cmd)) {
                    self.emit_event(event);
                }
            }
            _ => {
                trace!(command_id = command.command_id, tag = command.get_command_name(), "ignored");
            }
        }

        if !self.data.initialized && self.is_finishable() {
            self.data.initialized = true;
            info!("finished initialization");

            #[cfg(feature = "stream")]
            self.emit_initial_scan();
        }
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn is_initialized(&self) -> bool {
        self.data.initialized
    }

    #[instrument(skip_all)]
    fn export(&self) -> Option<Self::Export> {
        if self.is_empty() {
            warn!("no data was recorded");
            return None;
        }

        if self.data.trailblazer.is_none() {
            warn!("trailblazer gender was not recorded");
        }

        if self.data.uid.is_none() {
            warn!("uid was not recorded");
        }

        if self.data.relics.is_empty() {
            warn!("relics were not recorded");
        }

        if self.data.light_cones.is_empty() {
            warn!("light cones were not recorded");
        }

        if self.data.characters.is_empty() {
            warn!("characters were not recorded");
        }

        if !self.data.unresolved_multipath_characters.is_empty() {
            warn!(
                num = self.data.unresolved_multipath_characters.len(),
                "multipath characters were not resolved"
            );
        }

        let export = Export {
            source: "reliquary_archiver",
            build: env!("CARGO_PKG_VERSION"),
            version: 4,
            metadata: Metadata {
                uid: self.data.uid,
                trailblazer: self.data.trailblazer,
            },
            gacha: self.data.gacha,
            materials: self.data.materials.values().cloned().collect(),
            light_cones: self.data.light_cones.values().cloned().collect(),
            relics: self.data.relics.values().cloned().collect(),
            characters: self.data.characters.values().cloned().collect(),
            recon_relics: self.data.recon_relics.clone(),
        };

        Some(export)
    }

    #[cfg(feature = "stream")]
    fn subscribe(&self) -> (Option<OptimizerEvent>, broadcast::Receiver<OptimizerEvent>) {
        (
            if self.is_initialized() {
                Some(OptimizerEvent::InitialScan(
                    self.export().expect("marked as initialized but data was not recorded"),
                ))
            } else {
                None
            },
            self.event_channel.subscribe(),
        )
    }
}
