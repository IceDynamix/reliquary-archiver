use std::collections::{BTreeMap, HashMap, HashSet};

use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::{GameCommand, command_id};
#[cfg(feature = "stream")]
use tokio::sync::broadcast;
use tracing::{debug, info, instrument, trace, warn};

use crate::export::Exporter;
use crate::export::database::{Database, get_database};
use crate::export::fribbels::models::*;

pub struct OptimizerExporter {
    pub(super) database: &'static Database,

    // State fields
    pub(super) initialized: bool,
    pub(super) uid: Option<u32>,
    pub(super) trailblazer: Option<&'static str>,
    pub(super) banners: HashMap<u32, BannerInfo>,
    pub(super) gacha: GachaFunds,
    pub(super) materials: BTreeMap<u32, Material>,
    pub(super) light_cones: BTreeMap<u32, LightCone>,
    pub(super) relics: BTreeMap<u32, Relic>,
    /// relic recon relics keep a stable id throughout the event
    pub(super) recon_relics: Vec<u32>,
    /// All known characters. Level and ascension are only filled in once the base avatar is known
    pub(super) characters: BTreeMap<u32, Character>,
    pub(super) multipath_base_avatars: HashMap<u32, ProtoCharacter>,
    /// Characters in `characters` that are still waiting for their base avatar
    pub(super) unresolved_multipath_characters: HashSet<u32>,

    #[cfg(feature = "stream")]
    pub(super) event_channel: broadcast::Sender<OptimizerEvent>,
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

            // Initialize state fields
            initialized: false,
            uid: None,
            trailblazer: None,
            banners: HashMap::new(),
            gacha: GachaFunds::default(),
            materials: BTreeMap::new(),
            light_cones: BTreeMap::new(),
            relics: BTreeMap::new(),
            recon_relics: Vec::new(),
            characters: BTreeMap::new(),
            multipath_base_avatars: HashMap::new(),
            unresolved_multipath_characters: HashSet::new(),

            #[cfg(feature = "stream")]
            event_channel: broadcast::channel(16).0,
        }
    }

    /// The uid of the account, once it is known
    pub fn uid(&self) -> Option<u32> {
        self.uid
    }

    /// Which trailblazer the account plays, once it is known
    pub fn trailblazer(&self) -> Option<&'static str> {
        self.trailblazer
    }

    pub fn gacha_funds(&self) -> GachaFunds {
        self.gacha
    }

    /// The gacha banners by id
    pub fn banners(&self) -> &HashMap<u32, BannerInfo> {
        &self.banners
    }

    /// Materials by id
    pub fn materials(&self) -> &BTreeMap<u32, Material> {
        &self.materials
    }

    /// Light cones by unique id
    pub fn light_cones(&self) -> &BTreeMap<u32, LightCone> {
        &self.light_cones
    }

    /// Relics by unique id
    pub fn relics(&self) -> &BTreeMap<u32, Relic> {
        &self.relics
    }

    /// Characters by id.
    ///
    /// A multipath character's level and ascension come from its base avatar. Until that has been
    /// captured, they are 0, like in the export.
    pub fn characters(&self) -> &BTreeMap<u32, Character> {
        &self.characters
    }

    #[allow(unused_variables)]
    fn emit_event(&self, event: OptimizerEvent) {
        if self.initialized {
            // Send only fails if there are no active receivers. We don't care if this is the case.
            #[cfg(feature = "stream")]
            self.event_channel.send(event).ok();
        } else {
            // Don't start sending real-time updates until we've completed initialization.
        }
    }

    fn reset(&mut self) {
        self.initialized = false;
        self.uid = None;
        self.trailblazer = None;
        self.banners.clear();
        self.gacha = GachaFunds::default();
        self.materials.clear();
        self.light_cones.clear();
        self.relics.clear();
        self.characters.clear();
        self.multipath_base_avatars.clear();
        self.unresolved_multipath_characters.clear();
    }

    fn is_finishable(&self) -> bool {
        self.trailblazer.is_some()
            && self.uid.is_some()
            && !self.relics.is_empty()
            // at least one character has its level and ascension
            && self.characters.len() > self.unresolved_multipath_characters.len()
            && !self.light_cones.is_empty()
    }

    fn is_empty(&self) -> bool {
        self.trailblazer.is_none()
            && self.uid.is_none()
            && self.relics.is_empty()
            && self.characters.is_empty()
            && self.light_cones.is_empty()
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

        if !self.initialized && self.is_finishable() {
            self.initialized = true;
            info!("finished initialization");

            #[cfg(feature = "stream")]
            self.emit_initial_scan();
        }
    }

    fn is_empty(&self) -> bool {
        self.is_empty()
    }

    fn is_initialized(&self) -> bool {
        self.initialized
    }

    #[instrument(skip_all)]
    fn export(&self) -> Option<Self::Export> {
        if self.is_empty() {
            warn!("no data was recorded");
            return None;
        }

        if self.trailblazer.is_none() {
            warn!("trailblazer gender was not recorded");
        }

        if self.uid.is_none() {
            warn!("uid was not recorded");
        }

        if self.relics.is_empty() {
            warn!("relics were not recorded");
        }

        if self.light_cones.is_empty() {
            warn!("light cones were not recorded");
        }

        if self.characters.is_empty() {
            warn!("characters were not recorded");
        }

        if !self.unresolved_multipath_characters.is_empty() {
            warn!(
                num = self.unresolved_multipath_characters.len(),
                "multipath characters were not resolved"
            );
        }

        let export = Export {
            source: "reliquary_archiver",
            build: env!("CARGO_PKG_VERSION"),
            version: 4,
            metadata: Metadata {
                uid: self.uid,
                trailblazer: self.trailblazer,
            },
            gacha: self.gacha,
            materials: self.materials.values().cloned().collect(),
            light_cones: self.light_cones.values().cloned().collect(),
            relics: self.relics.values().cloned().collect(),
            characters: self.characters.values().cloned().collect(),
            recon_relics: self.recon_relics.clone(),
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
