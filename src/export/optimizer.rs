use std::collections::HashMap;
use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use reliquary::network::GameCommand;
use reliquary::network::gen::command_id;
use reliquary::network::gen::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::gen::proto::Equipment::Equipment as ProtoLightCone;
use reliquary::network::gen::proto::Gender::Gender;
use reliquary::network::gen::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::gen::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::gen::proto::GetHeroBasicTypeInfoScRsp::GetHeroBasicTypeInfoScRsp;
use reliquary::network::gen::proto::HeroBasicType::HeroBasicType;
use reliquary::network::gen::proto::HeroBasicTypeInfo::HeroBasicTypeInfo;
use reliquary::network::gen::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use reliquary::network::gen::proto::Relic::Relic as ProtoRelic;
use reliquary::network::gen::proto::AvatarSkillTree::AvatarSkillTree as ProtoSkillTree;
use reliquary::network::gen::proto::RelicAffix::RelicAffix;
use reliquary::resource::{ResourceMap};
use reliquary::resource::excel::*;
use reliquary::resource::text_map::TextMap;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use tracing::{debug, info, info_span, instrument, trace, warn};

use crate::export::Exporter;

const BASE_RESOURCE_URL: &str = "https://raw.githubusercontent.com/Dimbreath/StarRailData/master";

#[derive(Serialize, Deserialize, Debug)]
pub struct Export {
    pub source: &'static str,
    pub build: &'static str,
    pub version: u32,
    pub metadata: Metadata,
    pub light_cones: Vec<LightCone>,
    pub relics: Vec<Relic>,
    pub characters: Vec<Character>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Metadata {
    pub uid: Option<u32>,
    pub trailblazer: Option<&'static str>,
}

pub struct OptimizerExporter {
    database: Database,
    uid: Option<u32>,
    trailblazer: Option<&'static str>,
    light_cones: Vec<LightCone>,
    relics: Vec<Relic>,
    characters: Vec<Character>,
    trailblazer_characters: Vec<Character>,
}

impl OptimizerExporter {
    pub fn new(database: Database) -> OptimizerExporter {
        OptimizerExporter {
            database,
            uid: None,
            trailblazer: None,
            light_cones: vec![],
            relics: vec![],
            characters: vec![],
            trailblazer_characters: vec![],
        }
    }

    pub fn set_uid(&mut self, uid: String) {
        match uid.parse::<u32>() {
            Ok(uid) => { self.uid = Some(uid); }
            Err(e) => { warn!(e) }
        }
    }

    pub fn add_trailblazer_data(&mut self, hero: GetHeroBasicTypeInfoScRsp) {
        let gender = match hero.gender.enum_value().unwrap() {
            Gender::GenderNone => "", // probably in the prologue before selecting gender?
            Gender::GenderMan => "Caelus",
            Gender::GenderWoman => "Stelle"
        };

        self.trailblazer = Some(gender);
        info!(gender, "found trailblazer gender");

        let mut builds: Vec<Character> = hero.basic_type_info_list.iter()
            .filter_map(|b| export_proto_hero(&self.database, &b))
            .collect();

        info!(num=builds.len(), "found trailblazer builds");
        self.trailblazer_characters.append(&mut builds);
    }

    pub fn add_inventory(&mut self, bag: GetBagScRsp) {
        let mut relics: Vec<Relic> = bag.relic_list.iter()
            .filter_map(|r| export_proto_relic(&self.database, r))
            .collect();

        info!(num=relics.len(), "found relics");
        self.relics.append(&mut relics);

        let mut light_cones: Vec<LightCone> = bag.equipment_list.iter()
            .filter_map(|equip| export_proto_light_cone(&self.database, equip))
            .collect();

        info!(num=light_cones.len(), "found light cones");
        self.light_cones.append(&mut light_cones);
    }

    pub fn add_characters(&mut self, characters: GetAvatarDataScRsp) {
        let mut characters: Vec<Character> = characters.avatar_list.iter()
            .filter(|a| a.base_avatar_id < 8000) // skip trailblazer, handled in `write_hero`
            .filter_map(|char| export_proto_character(&self.database, char))
            .collect();

        info!(num=characters.len(), "found characters");
        self.characters.append(&mut characters);
    }
}

impl Exporter for OptimizerExporter {
    type Export = Export;

    fn read_command(&mut self, command: GameCommand) {
        match command.command_id {
            command_id::PlayerGetTokenScRsp => {
                debug!("detected uid");
                self.set_uid(
                    command.parse_proto::<PlayerGetTokenScRsp>().unwrap().uid.to_string()
                )
            }
            command_id::GetBagScRsp => {
                debug!("detected inventory packet");
                self.add_inventory(
                    command.parse_proto::<GetBagScRsp>().unwrap()
                )
            }
            command_id::GetAvatarDataScRsp => {
                debug!("detected character packet");
                self.add_characters(
                    command.parse_proto::<GetAvatarDataScRsp>().unwrap()
                )
            }
            command_id::GetHeroBasicTypeInfoScRsp => {
                debug!("detected trailblazer packet");
                self.add_trailblazer_data(
                    command.parse_proto::<GetHeroBasicTypeInfoScRsp>().unwrap()
                )
            }
            _ => {
                trace!(command_id=command.command_id, tag=command.get_command_name(), "ignored");
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.trailblazer.is_some()
            && self.uid.is_some()
            && !self.relics.is_empty()
            && !self.characters.is_empty()
            && !self.trailblazer_characters.is_empty()
            && !self.light_cones.is_empty()
    }

    #[instrument(skip_all)]
    fn export(self) -> Self::Export {
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

        if self.trailblazer_characters.is_empty() {
            warn!("trailblazer characters were not recorded");
        }

        if self.characters.is_empty() {
            warn!("characters were not recorded");
        }

        Export {
            source: "reliquary_archiver",
            build: env!("CARGO_PKG_VERSION"),
            version: 3,
            metadata: Metadata {
                uid: self.uid,
                trailblazer: self.trailblazer,
            },
            light_cones: self.light_cones,
            relics: self.relics,
            characters: self.characters.into_iter()
                .chain(self.trailblazer_characters)
                .collect(),
        }
    }
}

pub struct Database {
    avatar_config: AvatarConfigMap,
    avatar_skill_tree_config: AvatarSkillTreeConfigMap,
    equipment_config: EquipmentConfigMap,
    relic_config: RelicConfigMap,
    relic_set_config: RelicSetConfigMap,
    relic_main_affix_config: RelicMainAffixConfigMap,
    relic_sub_affix_config: RelicSubAffixConfigMap,
    text_map: TextMap,
    keys: HashMap<u32, Vec<u8>>,
}

impl Database {
    #[instrument(name = "config_map")]
    pub fn new_from_online() -> Self {
        Database {
            avatar_config: Self::load_online_config(),
            avatar_skill_tree_config: Self::load_online_config(),
            equipment_config: Self::load_online_config(),
            relic_config: Self::load_online_config(),
            relic_set_config: Self::load_online_config(),
            relic_main_affix_config: Self::load_online_config(),
            relic_sub_affix_config: Self::load_online_config(),
            text_map: Self::load_online_text_map(),
            keys: Self::load_online_keys(),
        }
    }

    // TODO: new_from_source

    fn load_online_config<T: ResourceMap + DeserializeOwned>() -> T {
        Self::get::<T>(format!("{BASE_RESOURCE_URL}/ExcelOutput/{}", T::get_json_name()))
    }
    fn load_online_text_map() -> TextMap {
        Self::get(format!("{BASE_RESOURCE_URL}/TextMap/TextMapEN.json"))
    }

    fn load_online_keys() -> HashMap<u32, Vec<u8>> {
        let keys: HashMap<u32, String> = Self::get("https://raw.githubusercontent.com/tamilpp25/Iridium-SR/main/data/Keys.json".to_string());
        let mut keys_bytes = HashMap::new();

        for (k, v) in keys {
            keys_bytes.insert(k, BASE64_STANDARD.decode(v).unwrap());
        }

        keys_bytes
    }

    fn get<T: DeserializeOwned>(url: String) -> T {
        info!(url, "requesting from resource");
        ureq::get(&url)
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }

    pub fn keys(&self) -> &HashMap<u32, Vec<u8>> {
        &self.keys
    }

    fn lookup_avatar_name(&self, avatar_id: u32) -> Option<String> {
        if avatar_id == 0 {
            return None;
        }

        let cfg = self.avatar_config.get(&avatar_id)?;
        cfg.AvatarName.lookup(&self.text_map).map(|s| s.to_string())
    }
}

#[tracing::instrument(name = "relic", skip_all, fields(id = proto.tid))]
fn export_proto_relic(config: &Database, proto: &ProtoRelic) -> Option<Relic> {
    let relic_config = config.relic_config.get(&proto.tid)?;

    let set_config = config.relic_set_config.get(&relic_config.SetID)?;
    let main_affix_config = config.relic_main_affix_config.get(&relic_config.MainAffixGroup, &proto.main_affix_id).unwrap();

    let id = proto.unique_id.to_string();
    let level = proto.level;
    let lock = proto.is_protected;
    let discard = proto.is_discarded;
    let set = set_config.SetName.lookup(&config.text_map)
        .map(|s| s.to_string())
        .unwrap_or("".to_string());

    let slot = slot_type_to_export(&relic_config.Type);
    let rarity = relic_config.MaxLevel / 3;
    let mainstat = main_stat_to_export(&main_affix_config.Property).to_string();
    let location = config.lookup_avatar_name(proto.base_avatar_id).unwrap_or("".to_string());

    debug!(rarity, set, slot, slot, mainstat, location, "detected");

    let substats = proto.sub_affix_list.iter()
        .filter_map(|substat| export_substat(config, rarity, substat))
        .collect();


    Some(Relic {
        set,
        slot,
        rarity,
        level,
        mainstat,
        substats,
        location,
        lock,
        discard,
        _id: id,
    })
}

#[tracing::instrument(name = "substat", skip_all)]
fn export_substat(config: &Database, rarity: u32, substat: &RelicAffix) -> Option<Substat> {
    let cfg = config.relic_sub_affix_config.get(&rarity, &substat.affix_id)?;
    let key = sub_stat_to_export(&cfg.Property).to_string();

    let mut value = substat.cnt as f32 * *cfg.BaseValue
        + substat.step as f32 * *cfg.StepValue;

    if key.ends_with('_') {
        value *= 100.0;
    }

    trace!(key, value, "detected substat");

    Some(Substat {
        key,
        value,
    })
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Relic {
    pub set: String,
    pub slot: &'static str,
    pub rarity: u32,
    pub level: u32,
    pub mainstat: String,
    pub substats: Vec<Substat>,
    pub location: String,
    pub lock: bool,
    pub discard: bool,
    pub _id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Substat {
    key: String,
    value: f32,
}

fn slot_type_to_export(s: &str) -> &'static str {
    match s {
        "HEAD" => "Head",
        "HAND" => "Hands",
        "BODY" => "Body",
        "FOOT" => "Feet",
        "NECK" => "Planar Sphere",
        "OBJECT" => "Link Rope",
        _ => panic!("Unknown slot: {}", s)
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
        _ => panic!("Unknown main stat: {}", s)
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
        _ => { panic!("Unknown sub stat: {}", s) }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct LightCone {
    pub key: String,
    pub level: u32,
    pub ascension: u32,
    pub superimposition: u32,
    pub location: String,
    pub lock: bool,
    pub _id: String,
}

#[instrument(name = "light_cone", skip_all, fields(id = proto.tid))]
fn export_proto_light_cone(config: &Database, proto: &ProtoLightCone) -> Option<LightCone> {
    let cfg = config.equipment_config.get(&proto.tid)?;
    let key = cfg.EquipmentName.lookup(&config.text_map).map(|s| s.to_string())?;

    let level = proto.level;
    let superimposition = proto.rank;

    debug!(light_cone=key, level, superimposition, "detected");

    let location = config.lookup_avatar_name(proto.base_avatar_id)
        .unwrap_or("".to_string());

    Some(LightCone {
        key,
        level,
        ascension: proto.promotion,
        superimposition,
        location,
        lock: proto.is_protected,
        _id: proto.unique_id.to_string(),
    })
}

#[instrument(name = "character", skip_all, fields(id = proto.base_avatar_id))]
fn export_proto_character(config: &Database, proto: &ProtoCharacter) -> Option<Character> {
    let key = config.lookup_avatar_name(proto.base_avatar_id)?;

    let level = proto.level;
    let eidolon = proto.rank;

    debug!(character=key, level, eidolon, "detected");

    let (skills, traces) = export_skill_tree(config, &proto.skilltree_list);

    Some(Character {
        key,
        level,
        ascension: proto.promotion,
        eidolon,
        skills,
        traces,
    })
}

fn export_proto_hero(config: &Database, proto: &HeroBasicTypeInfo) -> Option<Character> {
    use HeroBasicType::*;

    let path = proto.basic_type.enum_value().ok()?;
    let path = match path {
        BoyWarrior | GirlWarrior => "Destruction",
        BoyKnight | GirlKnight => "Preservation",
        BoyRogue | GirlRogue => "Hunt",
        BoyMage | GirlMage => "Erudition",
        BoyShaman | GirlShaman => "Harmony",
        BoyWarlock | GirlWarlock => "Nihility",
        BoyPriest | GirlPriest => "Abundance",
        _ => {
            debug!(?path, "unknown path");
            return Option::None;
        }
    };

    let key = format!("Trailblazer#{path}");

    let span = info_span!("character", key);
    let _enter = span.enter();

    trace!(character=key, "detected");

    let (skills, traces) = export_skill_tree(config, &proto.skill_tree_list);

    // TODO: figure out where level/ascension is stored
    Some(Character {
        key,
        level: 0,
        ascension: 0,
        eidolon: proto.rank,
        skills,
        traces,
    })
}

fn export_skill_tree(config: &Database, proto: &[ProtoSkillTree]) -> (Skills, Traces) {
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

    for skill in proto.iter().filter(|s| s.point_id != 0) {
        let level = skill.level;

        let span = info_span!("skill", id = skill.point_id, level);
        let _enter = span.enter();

        let Some(skill_tree_config) = config.avatar_skill_tree_config
            .get(&skill.point_id, &skill.level) else
        {
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

            _ => {
                warn!(anchor = skill_tree_config.Anchor, "unknown point anchor");
                continue;
            }
        }
    }

    (skills, traces)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Character {
    pub key: String,
    pub level: u32,
    pub ascension: u32,
    pub eidolon: u32,
    pub skills: Skills,
    pub traces: Traces,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Skills {
    pub basic: u32,
    pub skill: u32,
    pub ult: u32,
    pub talent: u32,
}

#[derive(Serialize, Deserialize, Debug)]
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