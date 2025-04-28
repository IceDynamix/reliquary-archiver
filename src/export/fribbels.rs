//! Output format based on the format used by [Fribbels HSR Optimizer],
//! devised by [kel-z's HSR-Scanner].
//!
//! [Fribbels HSR Optimizer]: https://github.com/fribbels/hsr-optimizer
//! [kel-z's HSR-Scanner]: https://github.com/kel-z/HSR-Scanner
use std::collections::{BTreeMap, HashMap, HashSet};

use crate::export::database::Database;
use protobuf::Enum;
use reliquary::network::command::proto::DoGachaScRsp::DoGachaScRsp;
use reliquary::network::command::proto::GetGachaInfoScRsp::GetGachaInfoScRsp;
use reliquary::network::command::{command_id, proto::PlayerLoginScRsp::PlayerLoginScRsp};
use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::proto::AvatarSkillTree::AvatarSkillTree as ProtoSkillTree;
use reliquary::network::command::proto::Equipment::Equipment as ProtoLightCone;
use reliquary::network::command::proto::Material::Material as ProtoMaterial;
use reliquary::network::command::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::command::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::command::proto::GetMultiPathAvatarInfoScRsp::GetMultiPathAvatarInfoScRsp;
use reliquary::network::command::proto::MultiPathAvatarType::MultiPathAvatarType;
use reliquary::network::command::proto::MultiPathAvatarTypeInfo::MultiPathAvatarTypeInfo;
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use reliquary::network::command::proto::PlayerSyncScNotify::PlayerSyncScNotify;
use reliquary::network::command::proto::Relic::Relic as ProtoRelic;
use reliquary::network::command::proto::RelicAffix::RelicAffix;
use reliquary::network::command::GameCommand;
use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};
use tracing::{debug, info, info_span, instrument, trace, warn};

#[cfg(feature = "stream")]
use crate::websocket;

use crate::export::Exporter;

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Export {
    pub source: &'static str,
    pub build: &'static str,
    pub version: u32,
    pub metadata: Metadata,
    pub gacha: GachaFunds,
    pub materials: Vec<Material>,
    pub light_cones: Vec<LightCone>,
    pub relics: Vec<Relic>,
    pub characters: Vec<Character>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Metadata {
    pub uid: Option<u32>,
    pub trailblazer: Option<&'static str>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct GachaFunds {
    pub stellar_jade: u32,
    pub oneric_shards: u32,
}

#[derive(Serialize, Debug, Clone, Copy)]
pub enum BannerType {
    Character,
    LightCone,
    Standard,
}

#[derive(Debug, Clone)]
pub struct BannerInfo {
    pub rate_up_item_list: Vec<u32>,
    pub banner_type: BannerType,
}

pub struct OptimizerExporter {
    database: Database,

    initialized: bool,
    uid: Option<u32>,
    trailblazer: Option<&'static str>,
    banners: BTreeMap<u32, BannerInfo>,
    gacha: GachaFunds,
    materials: BTreeMap<u32, Material>,
    light_cones: BTreeMap<u32, LightCone>,
    relics: BTreeMap<u32, Relic>,
    characters: BTreeMap<u32, Character>,
    multipath_characters: BTreeMap<u32, Character>,
    multipath_base_avatars: HashMap<u32, ProtoCharacter>,
    unresolved_multipath_characters: HashSet<u32>,

    #[cfg(feature = "stream")]
    websocket_tx: Option<websocket::ClientSender>,
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(tag = "event", content = "data")]
pub enum OptimizerEvent {
    InitialScan(Export),
    GachaResult(GachaResult),
    UpdateGachaFunds(GachaFunds),
    UpdateMaterials(Vec<Material>),
    UpdateLightCones(Vec<LightCone>),
    UpdateRelics(Vec<Relic>),
    UpdateCharacters(Vec<Character>),
    DeleteRelics(#[serde_as(as = "Vec<DisplayFromStr>")] Vec<u32>),
    DeleteLightCones(#[serde_as(as = "Vec<DisplayFromStr>")] Vec<u32>),
}

#[derive(Serialize, Debug, Clone)]
pub struct GachaResult {
    pub banner_id: u32,
    pub banner_type: BannerType,
    pub pity_4: PityUpdate,
    pub pity_5: PityUpdate,
    pub pull_results: Vec<u32>,
}

#[derive(Serialize, Debug, Clone, Copy)]
#[serde(tag = "kind")]
pub enum PityUpdate {
    AddPity {
        amount: u32,
    },
    ResetPity {
        amount: u32,
        set_guarantee: bool,
    },
}

impl PityUpdate {
    pub fn increment(&mut self) {
        match self {
            PityUpdate::AddPity { amount, .. } => *amount += 1,
            PityUpdate::ResetPity { amount, .. } => *amount += 1,
        }
    }

    pub fn reset(&mut self, guarantee: bool) {
        match self {
            PityUpdate::AddPity { amount } => {
                // Convert to the other variant.
                *self = PityUpdate::ResetPity {
                    amount: *amount,
                    set_guarantee: guarantee,
                }
            },
            PityUpdate::ResetPity { amount, set_guarantee, .. } => {
                *amount = 0;
                *set_guarantee = guarantee;
            },
        }
    }
}

impl OptimizerExporter {
    pub fn new(database: Database) -> OptimizerExporter {
        OptimizerExporter {
            database,

            initialized: false,
            uid: None,
            trailblazer: None,
            banners: BTreeMap::new(),
            gacha: GachaFunds::default(),
            materials: BTreeMap::new(),
            light_cones: BTreeMap::new(),
            relics: BTreeMap::new(),
            characters: BTreeMap::new(),
            multipath_characters: BTreeMap::new(),
            multipath_base_avatars: HashMap::new(),
            unresolved_multipath_characters: HashSet::new(),

            #[cfg(feature = "stream")]
            websocket_tx: None,
        }
    }

    fn is_finishable(&self) -> bool {
        self.trailblazer.is_some()
            && self.uid.is_some()
            && !self.relics.is_empty()
            && !self.characters.is_empty()
            && !self.multipath_characters.is_empty()
            && !self.light_cones.is_empty()
    }

    fn emit_event(&self, _event: OptimizerEvent) {
        #[cfg(feature = "stream")]
        if let Some(tx) = &self.websocket_tx {
            if self.initialized {
                websocket::broadcast_message(tx, _event);
            } else {
                // Don't start sending real-time updates until we've completed initialization.
            }
        }
    }

    fn reset(&mut self) {
        self.uid = None;
        self.trailblazer = None;
        self.light_cones.clear();
        self.relics.clear();
        self.characters.clear();
        self.multipath_characters.clear();
        self.multipath_base_avatars.clear();
        self.unresolved_multipath_characters.clear();
        self.initialized = false;
    }

    fn emit_initial_scan(&self) {
        #[cfg(feature = "stream")]
        if self.websocket_tx.is_some() {
            let export = self.export().expect("initial scan failed");
            self.emit_event(OptimizerEvent::InitialScan(export));
        }
    }

    pub fn set_uid(&mut self, uid: u32) {
        self.uid = Some(uid);
    }

    pub fn set_currency_count(&mut self, login: PlayerLoginScRsp) {
        self.gacha.oneric_shards = login.basic_info.oneric_shard_count;
        self.gacha.stellar_jade = login.basic_info.stellar_jade_count;
    }

    pub fn add_inventory(&mut self, bag: GetBagScRsp) {
        let relics: Vec<Relic> = bag
            .relic_list
            .iter()
            .filter_map(|r| export_proto_relic(&self.database, r))
            .collect();

        info!(num = relics.len(), "found relics");
        for relic in &relics {
            self.relics.insert(relic._uid, relic.clone());
        }

        let light_cones: Vec<LightCone> = bag
            .equipment_list
            .iter()
            .filter_map(|equip| export_proto_light_cone(&self.database, equip))
            .collect();

        info!(num = light_cones.len(), "found light cones");
        for light_cone in light_cones {
            self.light_cones.insert(light_cone._uid, light_cone);
        }

        let materials: Vec<Material> = bag
            .material_list
            .iter()
            .filter_map(|m| export_proto_material(&self.database, m))
            .collect();

        info!(num = materials.len(), "found materials");
        for material in materials {
            self.materials.insert(material.id, material);
        }
    }

    pub fn ingest_character(&mut self, proto_character: &ProtoCharacter) -> Option<Character> {
        let character = export_proto_character(&self.database, proto_character).unwrap();
        self.characters.insert(character.id, character.clone());

        if MultiPathAvatarType::from_i32(proto_character.base_avatar_id as i32).is_some() {
            self.multipath_base_avatars.insert(proto_character.base_avatar_id, proto_character.clone());

            // Try to resolve any multipath characters that have this as their base avatar.
            // TODO: Optimize by changing it to a map, keyed by base avatar id.
            for unresolved_avatar_id in self.unresolved_multipath_characters.clone().iter() {
                self.resolve_multipath_character(*unresolved_avatar_id);
            }

            return None;
        } else {
            // If the character is a multipath character, we need to wait for the 
            // multipath packet to get the rest of the data, so only emit character
            // here if it's not multipath.

            return Some(character);
        }
    }

    pub fn ingest_multipath_character(&mut self, proto_multipath_character: &MultiPathAvatarTypeInfo) -> Option<Character> {
        let character = export_proto_multipath_character(&self.database, proto_multipath_character).unwrap();
        self.multipath_characters.insert(character.id, character.clone());

        // If it's the trailblazer, determine the gender
        if character.name == "Trailblazer" {
            self.trailblazer = Some(if character.id % 2 == 0 {
                "Stelle"
            } else {
                "Caelus"
            });
        }

        if let Some(character) = self.resolve_multipath_character(character.id) {
            return Some(character);
        } else {
            debug!(uid = &character.id, "multipath character not resolved");
            self.unresolved_multipath_characters.insert(character.id);

            return None;
        }
    }

    pub fn add_characters(&mut self, characters: GetAvatarDataScRsp) {
        info!(num = characters.avatar_list.len(), "found characters");
        for character in characters.avatar_list {
            self.ingest_character(&character);
        }
    }

    pub fn add_multipath_characters(&mut self, characters: GetMultiPathAvatarInfoScRsp) {
        info!(num = characters.multi_path_avatar_type_info_list.len(), "found multipath characters");
        for multipath_avatar_info in characters.multi_path_avatar_type_info_list {
            self.ingest_multipath_character(&multipath_avatar_info);
        }
    }

    fn get_multipath_base_id(&self, avatar_id: u32) -> u32 {
        self
            .database
            .multipath_avatar_config
            .get(&avatar_id)
            .expect("multipath character not found")
            .BaseAvatarID
    }

    pub fn resolve_multipath_character(&mut self, character_id: u32) -> Option<Character> {
        let base_avatar_id = self.get_multipath_base_id(character_id);
        let character = self.multipath_characters.get_mut(&character_id).unwrap();
        if let Some(base_avatar) = self.multipath_base_avatars.get(&base_avatar_id) {
            character.level = base_avatar.level;
            character.ascension = base_avatar.promotion;

            self.unresolved_multipath_characters.remove(&character_id);

            return Some(character.clone());
        }

        return None;
    }

    pub fn process_player_sync(&mut self, sync: PlayerSyncScNotify) {
        let relics: Vec<Relic> = sync
            .relic_list
            .iter()
            .filter_map(|r| export_proto_relic(&self.database, r))
            .collect();

        if !relics.is_empty() {
            info!(num = relics.len(), "found updated relics");
            for relic in relics.clone() {
                self.relics.insert(relic._uid, relic);
            }

            self.emit_event(OptimizerEvent::UpdateRelics(relics));
        }

        let light_cones: Vec<LightCone> = sync
            .equipment_list
            .iter()
            .filter_map(|equip| export_proto_light_cone(&self.database, equip))
            .collect();

        if !light_cones.is_empty() {
            info!(num = light_cones.len(), "found updated light cones");
            for light_cone in light_cones.clone() {
                self.light_cones.insert(light_cone._uid, light_cone);
            }

            self.emit_event(OptimizerEvent::UpdateLightCones(light_cones));
        }

        let materials: Vec<Material> = sync
            .material_list
            .iter()
            .filter_map(|m| export_proto_material(&self.database, m))
            .collect();

        if !materials.is_empty() {
            info!(num = materials.len(), "found updated materials");
            for material in materials.clone() {
                self.materials.insert(material.id, material);
            }

            self.emit_event(OptimizerEvent::UpdateMaterials(materials));
        }

        if let Some(basic_info) = sync.basic_info.into_option() {
            self.gacha.oneric_shards = basic_info.oneric_shard_count;
            self.gacha.stellar_jade = basic_info.stellar_jade_count;

            self.emit_event(OptimizerEvent::UpdateGachaFunds(self.gacha.clone()));
        }

        if !sync.del_relic_list.is_empty() {
            info!(num = sync.del_relic_list.len(), "found deleted relics");
            for del_relic in sync.del_relic_list.iter() {
                if self.relics.remove(del_relic).is_none() {
                    warn!(uid = &del_relic, "del_relic not found");
                }
            }

            self.emit_event(OptimizerEvent::DeleteRelics(sync.del_relic_list));
        }

        if !sync.del_equipment_list.is_empty() {
            info!(num = sync.del_equipment_list.len(), "found deleted light cones");
            for del_light_cone in sync.del_equipment_list.iter() {
                if self.light_cones.remove(del_light_cone).is_none() {
                    warn!(uid = &del_light_cone, "del_light_cone not found");
                }
            }

            self.emit_event(OptimizerEvent::DeleteLightCones(sync.del_equipment_list));
        }

        let mut updated_characters = Vec::new();

        if let Some(avatar_sync) = sync.avatar_sync.into_option() {
            for avatar in avatar_sync.avatar_list {
                if let Some(character) = self.ingest_character(&avatar) {
                    updated_characters.push(character);
                }
            }
        }

        if !sync.multi_path_avatar_type_info_list.is_empty() {
            for multipath_avatar_info in sync.multi_path_avatar_type_info_list {
                if let Some(character) = self.ingest_multipath_character(&multipath_avatar_info) {
                    updated_characters.push(character);
                } else {
                    warn!(uid = &multipath_avatar_info.avatar_id.value(), "multipath character not resolved");
                }
            }
        }

        if !updated_characters.is_empty() {
            info!(num = updated_characters.len(), "found updated characters");
            self.emit_event(OptimizerEvent::UpdateCharacters(updated_characters));
        }
    }

    fn is_lightcone(&self, item_id: u32) -> bool {
        self.database.equipment_config.get(&item_id).is_some()
    }

    pub fn process_gacha_info(&mut self, gacha_info: GetGachaInfoScRsp) {
        for banner in gacha_info.gacha_info_list {
            self.banners.insert(banner.gacha_id, BannerInfo {
                rate_up_item_list: banner.item_detail_list,
                banner_type: match banner.gacha_id {
                    1001 => BannerType::Standard,
                    _ => {
                        if self.is_lightcone(*banner.prize_item_list.first().unwrap()) {
                            BannerType::LightCone
                        } else {
                            BannerType::Character
                        }
                    },
                },
            });
        }
    }

    pub fn process_gacha(&mut self, gacha: DoGachaScRsp) {
        if let Some(banner) = self.banners.get(&gacha.gacha_id) {
            let mut gacha_result = GachaResult {
                banner_id: gacha.gacha_id,
                banner_type: banner.banner_type,
                pity_4: PityUpdate::AddPity { amount: 0 },
                pity_5: PityUpdate::AddPity { amount: 0 },
                pull_results: Vec::new(),
            };

            for item in gacha.gacha_item_list {
                gacha_result.pull_results.push(item.gacha_item.item_id);

                let grade = if let Some(lc_config) = self.database.equipment_config.get(&item.gacha_item.item_id) {
                    match lc_config.Rarity.as_str() {
                        "CombatPowerLightconeRarity5" => 5,
                        "CombatPowerLightconeRarity4" => 4,
                        "CombatPowerLightconeRarity3" => 3,
                        _ => panic!("Unknown light cone rarity: {}", lc_config.Rarity),
                    }
                } else if let Some(avatar_config) = self.database.avatar_config.get(&item.gacha_item.item_id) {
                    match avatar_config.Rarity.as_str() {
                        "CombatPowerAvatarRarityType5" => 5,
                        "CombatPowerAvatarRarityType4" => 4,
                        _ => panic!("Unknown avatar rarity: {}", avatar_config.Rarity),
                    }
                } else {
                    panic!("item not found: {}", item.gacha_item.item_id);
                };

                let was_rate_up = banner.rate_up_item_list.contains(&item.gacha_item.item_id);
                let next_is_guarantee = !was_rate_up;

                match grade {
                    5 => {
                        gacha_result.pity_4.increment();
                        gacha_result.pity_5.reset(next_is_guarantee);
                    },
                    4 => {
                        gacha_result.pity_4.reset(next_is_guarantee);
                        gacha_result.pity_5.increment();
                    },
                    _ => {
                        gacha_result.pity_4.increment();
                        gacha_result.pity_5.increment();
                    }
                }
            }

            self.emit_event(OptimizerEvent::GachaResult(gacha_result));
        } else {
            warn!(gacha_id = &gacha.gacha_id, "gacha info not found");
        }
    }
}

impl Exporter for OptimizerExporter {
    type Export = Export;
    type LiveEvent = OptimizerEvent;

    fn read_command(&mut self, command: GameCommand) {
        match command.command_id {
            command_id::PlayerGetTokenScRsp => {
                info!("detected new login attempt, resetting state");
                self.reset();

                debug!("detected uid");
                let cmd = command.parse_proto::<PlayerGetTokenScRsp>();
                match cmd {
                    Ok(cmd) => self.set_uid(cmd.uid),
                    Err(error) => {
                        warn!(%error, "could not parse token command");
                    }
                }
            }
            command_id::PlayerLoginScRsp => {
                debug!("detected login info packet");
                let cmd = command.parse_proto::<PlayerLoginScRsp>();
                match cmd {
                    Ok(cmd) => self.set_currency_count(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse login info data command");
                    }
                }
            }
            command_id::GetBagScRsp => {
                debug!("detected inventory packet");
                let cmd = command.parse_proto::<GetBagScRsp>();
                match cmd {
                    Ok(cmd) => self.add_inventory(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse inventory data command");
                    }
                }
            }
            command_id::GetAvatarDataScRsp => {
                debug!("detected character packet");
                let cmd = command.parse_proto::<GetAvatarDataScRsp>();
                match cmd {
                    Ok(cmd) => self.add_characters(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse character data command");
                    }
                }
            }
            command_id::GetMultiPathAvatarInfoScRsp => {
                debug!("detected multipath packet (trailblazer/march 7th)");
                let cmd = command.parse_proto::<GetMultiPathAvatarInfoScRsp>();
                match cmd {
                    Ok(cmd) => self.add_multipath_characters(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse multipath data command");
                    }
                }
            }
            command_id::PlayerSyncScNotify => {
                debug!("detected player sync packet");
                let cmd = command.parse_proto::<PlayerSyncScNotify>();
                match cmd {
                    Ok(cmd) => self.process_player_sync(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse player sync data command");
                    }
                }
            }
            command_id::GetGachaInfoScRsp => {
                debug!("detected gacha info packet");
                let cmd = command.parse_proto::<GetGachaInfoScRsp>();
                match cmd {
                    Ok(cmd) => self.process_gacha_info(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse gacha info data command");
                    }
                }
            }
            command_id::DoGachaScRsp => {
                debug!("detected gacha packet");
                let cmd = command.parse_proto::<DoGachaScRsp>();
                match cmd {
                    Ok(cmd) => self.process_gacha(cmd),
                    Err(error) => {
                        warn!(%error, "could not parse gacha data command");
                    }
                }
            }
            _ => {
                trace!(
                    command_id = command.command_id,
                    tag = command.get_command_name(),
                    "ignored"
                );
            }
        }

        if !self.initialized && self.is_finishable() {
            self.initialized = true;
            info!("finished initialization");

            self.emit_initial_scan();
        }
    }

    fn is_empty(&self) -> bool {
        self.trailblazer.is_none()
            && self.uid.is_none()
            && self.relics.is_empty()
            && self.characters.is_empty()
            && self.multipath_characters.is_empty()
            && self.light_cones.is_empty()
    }

    fn is_finished(&self) -> bool {
        self.initialized
    }

    #[instrument(skip_all)]
    fn export(&self) -> Option<Self::Export> {
        info!("exporting collected data");

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
            warn!(num = self.unresolved_multipath_characters.len(), "multipath characters were not resolved");
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

    fn get_initial_event(&self) -> Option<OptimizerEvent> {
        if self.is_finished() {
            Some(OptimizerEvent::InitialScan(self.export().expect("marked as finished but data was not recorded")))
        } else {
            None
        }
    }

    #[cfg(feature = "stream")]
    fn set_streamer(&mut self, tx: Option<websocket::ClientSender>) {
        self.websocket_tx = tx;
    }
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Material {
    #[serde_as(as = "DisplayFromStr")]
    pub id: u32,
    pub name: String,
    pub count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<u64>,
}

#[instrument(name = "material", skip_all, fields(id = proto.tid))]
fn export_proto_material(db: &Database, proto: &ProtoMaterial) -> Option<Material> {
    let cfg = db.item_config.get(&proto.tid)?;
    let id = cfg.ID;
    let name = cfg
        .ItemName
        .map(|hash| db.text_map.get(&hash))
        .flatten()
        .map(|s| s.to_string())?;
    let count = proto.num;

    debug!(material = name, count, "detected");

    Some(Material {
        id,
        name,
        count,
        expire_time: None,
    })
}

fn format_location(avatar_id: u32) -> String {
    if avatar_id == 0 {
        "".to_owned()
    } else {
        avatar_id.to_string()
    }
}

#[tracing::instrument(name = "relic", skip_all, fields(id = proto.tid))]
fn export_proto_relic(db: &Database, proto: &ProtoRelic) -> Option<Relic> {
    let relic_config = db.relic_config.get(&proto.tid)?;

    let set_id = relic_config.SetID;
    let set_config = db.relic_set_config.get(&set_id)?;
    let main_affix_config = db
        .relic_main_affix_config
        .get(&relic_config.MainAffixGroup, &proto.main_affix_id)
        .unwrap();

    let id = proto.unique_id;
    let level = proto.level;
    let lock = proto.is_protected;
    let discard = proto.is_discarded;
    let set_name = set_config
        .SetName
        .lookup(&db.text_map)
        .map(|s| s.to_string())
        .unwrap_or("".to_string());

    let slot = slot_type_to_export(&relic_config.Type);
    let rarity = relic_config.MaxLevel / 3;
    let mainstat = main_stat_to_export(&main_affix_config.Property).to_string();
    let location = format_location(proto.equip_avatar_id);

    debug!(rarity, set_name, slot, slot, mainstat, location, "detected");

    let substats = proto
        .sub_affix_list
        .iter()
        .filter_map(|substat| export_substat(db, rarity, substat))
        .collect();

    Some(Relic {
        set_id,
        name: set_name,
        slot,
        rarity,
        level,
        mainstat,
        substats,
        location,
        lock,
        discard,
        _uid: id,
    })
}

#[tracing::instrument(name = "substat", skip_all)]
fn export_substat(db: &Database, rarity: u32, substat: &RelicAffix) -> Option<Substat> {
    let cfg = db.relic_sub_affix_config.get(&rarity, &substat.affix_id)?;
    let key = sub_stat_to_export(&cfg.Property).to_string();

    let mut value = substat.cnt as f32 * *cfg.BaseValue + substat.step as f32 * *cfg.StepValue;

    if key.ends_with('_') {
        value *= 100.0;
    }

    trace!(key, value, "detected substat");

    Some(Substat {
        key,
        value,
        count: substat.cnt,
        step: substat.step,
    })
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Relic {
    #[serde_as(as = "DisplayFromStr")]
    pub set_id: u32,
    pub name: String,
    pub slot: &'static str,
    pub rarity: u32,
    pub level: u32,
    pub mainstat: String,
    pub substats: Vec<Substat>,
    pub location: String,
    pub lock: bool,
    pub discard: bool,
    #[serde_as(as = "DisplayFromStr")]
    pub _uid: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Substat {
    key: String,
    value: f32,
    count: u32,
    step: u32,
}

fn slot_type_to_export(s: &str) -> &'static str {
    match s {
        "HEAD" => "Head",
        "HAND" => "Hands",
        "BODY" => "Body",
        "FOOT" => "Feet",
        "NECK" => "Planar Sphere",
        "OBJECT" => "Link Rope",
        _ => panic!("Unknown slot: {}", s),
    }
}

fn main_stat_to_export(s: &str) -> &'static str {
    match s {
        "HPDelta" => "HP",
        "AttackDelta" => "ATK",
        "HPAddedRatio" => "HP",
        "AttackAddedRatio" => "ATK",
        "DefenceAddedRatio" => "DEF",
        "CriticalChanceBase" => "CRIT Rate",
        "CriticalDamageBase" => "CRIT DMG",
        "HealRatioBase" => "Outgoing Healing Boost",
        "SpeedDelta" => "SPD",
        "StatusProbabilityBase" => "Effect Hit Rate",
        "PhysicalAddedRatio" => "Physical DMG Boost",
        "FireAddedRatio" => "Fire DMG Boost",
        "IceAddedRatio" => "Ice DMG Boost",
        "ThunderAddedRatio" => "Lightning DMG Boost",
        "WindAddedRatio" => "Wind DMG Boost",
        "QuantumAddedRatio" => "Quantum DMG Boost",
        "ImaginaryAddedRatio" => "Imaginary DMG Boost",
        "BreakDamageAddedRatioBase" => "Break Effect",
        "SPRatioBase" => "Energy Regeneration Rate",
        _ => panic!("Unknown main stat: {}", s),
    }
}

fn sub_stat_to_export(s: &str) -> &'static str {
    match s {
        "HPDelta" => "HP",
        "AttackDelta" => "ATK",
        "HPAddedRatio" => "HP_",
        "AttackAddedRatio" => "ATK_",
        "DefenceAddedRatio" => "DEF_",
        "DefenceDelta" => "DEF",
        "CriticalChanceBase" => "CRIT Rate_",
        "CriticalDamageBase" => "CRIT DMG_",
        "SpeedDelta" => "SPD",
        "StatusProbabilityBase" => "Effect Hit Rate_",
        "StatusResistanceBase" => "Effect RES_",
        "BreakDamageAddedRatioBase" => "Break Effect_",
        _ => {
            panic!("Unknown sub stat: {}", s)
        }
    }
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LightCone {
    #[serde_as(as = "DisplayFromStr")]
    pub id: u32,
    pub name: String,
    pub level: u32,
    pub ascension: u32,
    pub superimposition: u32,
    pub location: String,
    pub lock: bool,
    #[serde_as(as = "DisplayFromStr")]
    pub _uid: u32,
}

#[instrument(name = "light_cone", skip_all, fields(id = proto.tid))]
fn export_proto_light_cone(db: &Database, proto: &ProtoLightCone) -> Option<LightCone> {
    let cfg = db.equipment_config.get(&proto.tid)?;
    let id = cfg.EquipmentID;
    let name = cfg
        .EquipmentName
        .lookup(&db.text_map)
        .map(|s| s.to_string())?;

    let level = proto.level;
    let superimposition = proto.rank;

    debug!(light_cone = name, level, superimposition, "detected");

    let location = format_location(proto.equip_avatar_id);

    Some(LightCone {
        id,
        name,
        level,
        ascension: proto.promotion,
        superimposition,
        location,
        lock: proto.is_protected,
        _uid: proto.unique_id,
    })
}

#[instrument(name = "character", skip_all, fields(id = proto.base_avatar_id))]
fn export_proto_character(db: &Database, proto: &ProtoCharacter) -> Option<Character> {
    let id = proto.base_avatar_id;
    let name = db.lookup_avatar_name(id)?;
    let path = avatar_path_lookup(db, id)?.to_owned();

    let level = proto.level;
    let eidolon = proto.rank;

    debug!(character = name, level, eidolon, "detected");

    let (skills, traces, memosprite) = export_skill_tree(db, &proto.avatar_skilltree_list);

    Some(Character {
        id,
        name,
        path,
        level,
        ascension: proto.promotion,
        eidolon,
        skills,
        traces,
        memosprite,
    })
}

fn export_proto_multipath_character(
    db: &Database,
    proto: &MultiPathAvatarTypeInfo,
) -> Option<Character> {
    let id = proto.avatar_id.value() as u32;
    let name = db.lookup_avatar_name(id)?;
    let path = avatar_path_lookup(db, id)?.to_owned();

    let span = info_span!("character", name, path);
    let _enter = span.enter();

    trace!(character = name, path, "detected");

    let (skills, traces, memosprite) = export_skill_tree(db, &proto.multipath_skilltree_list);

    Some(Character {
        id,
        name,
        path,
        // Level and ascension are stored in the base avatar
        // in the main character list, set them to 0 for now.
        // Will be updated in [finalize_multipath_characters]
        level: 0,
        ascension: 0,
        eidolon: proto.rank,
        skills,
        traces,
        memosprite,
    })
}

fn avatar_path_lookup(db: &Database, avatar_id: u32) -> Option<&'static str> {
    let hero_config = db.avatar_config.get(&avatar_id);
    let avatar_base_type = hero_config.unwrap().AvatarBaseType.as_str();
    match avatar_base_type {
        "Knight" => Some("Preservation"),
        "Rogue" => Some("Hunt"),
        "Mage" => Some("Erudition"),
        "Warlock" => Some("Nihility"),
        "Warrior" => Some("Destruction"),
        "Shaman" => Some("Harmony"),
        "Priest" => Some("Abundance"),
        "Memory" => Some("Remembrance"),
        _ => {
            debug!(?avatar_base_type, "unknown path");
            None
        }
    }
}

fn export_skill_tree(db: &Database, proto: &[ProtoSkillTree]) -> (Skills, Traces, Option<Memosprite>) {
    let mut skills = Skills {
        basic: 0,
        skill: 0,
        ult: 0,
        talent: 0,
    };

    let mut traces = Traces {
        ability_1: false,
        ability_2: false,
        ability_3: false,
        stat_1: false,
        stat_2: false,
        stat_3: false,
        stat_4: false,
        stat_5: false,
        stat_6: false,
        stat_7: false,
        stat_8: false,
        stat_9: false,
        stat_10: false,
    };

    let mut memosprite = Memosprite {
        skill: 0,
        talent: 0,
    };

    for skill in proto.iter().filter(|s| s.point_id != 0) {
        let level = skill.level;

        let span = info_span!("skill", id = skill.point_id, level);
        let _enter = span.enter();

        let Some(skill_tree_config) = db
            .avatar_skill_tree_config
            .get(&skill.point_id, &skill.level)
        else {
            warn!("could not look up skill tree config");
            continue;
        };

        match skill_tree_config.Anchor.as_str() {
            "Point01" => {
                trace!(level, "detected basic atk trace");
                skills.basic = level;
            }
            "Point02" => {
                trace!(level, "detected skill trace");
                skills.skill = level;
            }
            "Point03" => {
                trace!(level, "detected ult trace");
                skills.ult = level;
            }
            "Point04" => {
                trace!(level, "detected talent trace");
                skills.talent = level;
            }

            "Point05" => {
                trace!(level, "detected technique trace");
                /* technique */
            }

            "Point06" => {
                trace!("detected major trace 1");
                traces.ability_1 = true;
            }
            "Point07" => {
                trace!("detected major trace 2");
                traces.ability_2 = true;
            }
            "Point08" => {
                trace!("detected major trace 3");
                traces.ability_3 = true;
            }

            "Point09" => {
                trace!("detected minor trace 1");
                traces.stat_1 = true;
            }
            "Point10" => {
                trace!("detected minor trace 2");
                traces.stat_2 = true;
            }
            "Point11" => {
                trace!("detected minor trace 3");
                traces.stat_3 = true;
            }
            "Point12" => {
                trace!("detected minor trace 4");
                traces.stat_4 = true;
            }
            "Point13" => {
                trace!("detected minor trace 5");
                traces.stat_5 = true;
            }
            "Point14" => {
                trace!("detected minor trace 6");
                traces.stat_6 = true;
            }
            "Point15" => {
                trace!("detected minor trace 7");
                traces.stat_7 = true;
            }
            "Point16" => {
                trace!("detected minor trace 8");
                traces.stat_8 = true;
            }
            "Point17" => {
                trace!("detected minor trace 9");
                traces.stat_9 = true;
            }
            "Point18" => {
                trace!("detected minor trace 10");
                traces.stat_10 = true;
            }
            "Point19" => {
                trace!("detected memosprite skill trace");
                memosprite.skill = level;
            }
            "Point20" => {
                trace!("detected memosprite talent trace");
                memosprite.talent = level;
            }

            _ => {
                warn!(anchor = skill_tree_config.Anchor, "unknown point anchor");
                continue;
            }
        }
    }

    (skills, traces, memosprite.if_present())
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Character {
    #[serde_as(as = "DisplayFromStr")]
    pub id: u32,
    pub name: String,
    pub path: String,
    pub level: u32,
    pub ascension: u32,
    pub eidolon: u32,
    pub skills: Skills,
    pub traces: Traces,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memosprite: Option<Memosprite>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Skills {
    pub basic: u32,
    pub skill: u32,
    pub ult: u32,
    pub talent: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Traces {
    pub ability_1: bool,
    pub ability_2: bool,
    pub ability_3: bool,
    pub stat_1: bool,
    pub stat_2: bool,
    pub stat_3: bool,
    pub stat_4: bool,
    pub stat_5: bool,
    pub stat_6: bool,
    pub stat_7: bool,
    pub stat_8: bool,
    pub stat_9: bool,
    pub stat_10: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Memosprite {
    pub skill: u32,
    pub talent: u32,
}

impl Memosprite {
    fn if_present(self) -> Option<Memosprite> {
        if self.skill == 0 && self.talent == 0 {
            None
        } else {
            Some(self)
        }
    }
}
