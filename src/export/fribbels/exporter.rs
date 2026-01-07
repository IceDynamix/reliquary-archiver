use std::collections::{BTreeMap, HashMap, HashSet};

use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::proto::DoGachaScRsp::DoGachaScRsp;
use reliquary::network::command::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::command::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::command::proto::GetGachaInfoScRsp::GetGachaInfoScRsp;
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use reliquary::network::command::proto::PlayerLoginScRsp::PlayerLoginScRsp;
use reliquary::network::command::proto::PlayerSyncScNotify::PlayerSyncScNotify;
use reliquary::network::command::proto::SetAvatarEnhancedIdScRsp::SetAvatarEnhancedIdScRsp;
use reliquary::network::command::{command_id, GameCommand};
#[cfg(feature = "stream")]
use tokio::sync::broadcast;
use tracing::{debug, info, instrument, trace, warn};

use crate::export::database::{get_database, Database};
use crate::export::fribbels::models::*;
use crate::export::Exporter;

pub struct OptimizerExporter {
    pub(super) database: &'static Database,

    // State fields
    pub initialized: bool,
    pub uid: Option<u32>,
    pub trailblazer: Option<&'static str>,
    pub banners: HashMap<u32, BannerInfo>,
    pub gacha: GachaFunds,
    pub materials: BTreeMap<u32, Material>,
    pub light_cones: BTreeMap<u32, LightCone>,
    pub relics: BTreeMap<u32, Relic>,
    pub characters: BTreeMap<u32, Character>,
    pub multipath_characters: BTreeMap<u32, Character>,
    pub multipath_base_avatars: HashMap<u32, ProtoCharacter>,
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
            characters: BTreeMap::new(),
            multipath_characters: BTreeMap::new(),
            multipath_base_avatars: HashMap::new(),
            unresolved_multipath_characters: HashSet::new(),

            #[cfg(feature = "stream")]
            event_channel: broadcast::channel(16).0,
        }
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
        self.multipath_characters.clear();
        self.multipath_base_avatars.clear();
        self.unresolved_multipath_characters.clear();
    }

    fn is_finishable(&self) -> bool {
        self.trailblazer.is_some()
            && self.uid.is_some()
            && !self.relics.is_empty()
            && !self.characters.is_empty()
            && !self.multipath_characters.is_empty()
            && !self.light_cones.is_empty()
    }

    fn is_empty(&self) -> bool {
        self.trailblazer.is_none()
            && self.uid.is_none()
            && self.relics.is_empty()
            && self.characters.is_empty()
            && self.multipath_characters.is_empty()
            && self.light_cones.is_empty()
    }

    #[cfg(feature = "stream")]
    fn emit_initial_scan(&self) {
        let export = self.export().expect("initial scan failed");
        self.emit_event(OptimizerEvent::InitialScan(export));
    }
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
                let cmd = command.parse_proto::<PlayerGetTokenScRsp>();
                match cmd {
                    Ok(cmd) => self.handle_token(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse token command");
                    }
                }
            }
            command_id::PlayerLoginScRsp => {
                debug!("detected login info packet");
                let cmd = command.parse_proto::<PlayerLoginScRsp>();
                match cmd {
                    Ok(cmd) => self.handle_login(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse login info data command");
                    }
                }
            }
            command_id::GetBagScRsp => {
                debug!("detected inventory packet");
                let cmd = command.parse_proto::<GetBagScRsp>();
                match cmd {
                    Ok(cmd) => self.handle_inventory(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse inventory data command");
                    }
                }
            }
            command_id::GetAvatarDataScRsp => {
                debug!("detected character packet");
                let cmd = command.parse_proto::<GetAvatarDataScRsp>();
                match cmd {
                    Ok(cmd) => self.handle_characters(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse character data command");
                    }
                }
            }
            command_id::SetAvatarEnhancedIdScRsp => {
                debug!("detected set avatar enhanced packet");
                let cmd = command.parse_proto::<SetAvatarEnhancedIdScRsp>();
                match cmd {
                    Ok(cmd) => {
                        let event = self.handle_set_avatar_enhanced(cmd);
                        self.emit_event(event);
                    }
                    Err(error) => {
                        warn!(%error, "could not parse set avatar enhanced data command");
                    }
                }
            }
            command_id::GetGachaInfoScRsp => {
                debug!("detected gacha info packet");
                let cmd = command.parse_proto::<GetGachaInfoScRsp>();
                match cmd {
                    Ok(cmd) => self.handle_gacha_info(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse gacha info data command");
                    }
                }
            }
            command_id::DoGachaScRsp => {
                debug!("detected gacha packet");
                let cmd = command.parse_proto::<DoGachaScRsp>();
                match cmd {
                    Ok(cmd) => {
                        if let Some(event) = self.handle_gacha(cmd) {
                            self.emit_event(event);
                        }
                    }
                    Err(error) => {
                        warn!(%error, "could not parse gacha data command");
                    }
                }
            }
            command_id::PlayerSyncScNotify => {
                debug!("detected player sync packet");
                let cmd = command.parse_proto::<PlayerSyncScNotify>();
                match cmd {
                    Ok(cmd) => {
                        let events = self.handle_player_sync(cmd);
                        for event in events {
                            self.emit_event(event);
                        }
                    }
                    Err(error) => {
                        warn!(%error, "could not parse player sync data command");
                    }
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

        if self.multipath_characters.is_empty() {
            warn!("multipath characters were not recorded");
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
            characters: self
                .characters
                .iter()
                .chain(self.multipath_characters.iter())
                .map(|(_id, c)| c.clone()) // Discard the key
                .collect(),
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
