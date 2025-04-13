//! Output format based on the format used by [Fribbels HSR Optimizer],
//! devised by [kel-z's HSR-Scanner].
//!
//! [Fribbels HSR Optimizer]: https://github.com/fribbels/hsr-optimizer
//! [kel-z's HSR-Scanner]: https://github.com/kel-z/HSR-Scanner
use std::collections::HashMap;

use crate::export::database::Database;
use protobuf::Enum;
use reliquary::network::command::command_id;
use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::proto::AvatarSkillTree::AvatarSkillTree as ProtoSkillTree;
use reliquary::network::command::proto::Equipment::Equipment as ProtoLightCone;
use reliquary::network::command::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::command::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::command::proto::GetMultiPathAvatarInfoScRsp::GetMultiPathAvatarInfoScRsp;
use reliquary::network::command::proto::MultiPathAvatarType::MultiPathAvatarType;
use reliquary::network::command::proto::MultiPathAvatarTypeInfo::MultiPathAvatarTypeInfo;
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use reliquary::network::command::proto::Relic::Relic as ProtoRelic;
use reliquary::network::command::proto::RelicAffix::RelicAffix;
use reliquary::network::command::GameCommand;
use serde::{Deserialize, Serialize};
use tracing::{debug, info, info_span, instrument, trace, warn};

use crate::export::Exporter;

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
    multipath_characters: Vec<Character>,
    multipath_base_avatars: HashMap<u32, ProtoCharacter>,
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
            multipath_characters: vec![],
            multipath_base_avatars: HashMap::new(),
        }
    }

    pub fn set_uid(&mut self, uid: u32) {
        self.uid = Some(uid);
    }

    pub fn add_inventory(&mut self, bag: GetBagScRsp) {
        let mut relics: Vec<Relic> = bag
            .relic_list
            .iter()
            .filter_map(|r| export_proto_relic(&self.database, r))
            .collect();

        info!(num = relics.len(), "found relics");
        self.relics.append(&mut relics);

        let mut light_cones: Vec<LightCone> = bag
            .equipment_list
            .iter()
            .filter_map(|equip| export_proto_light_cone(&self.database, equip))
            .collect();

        info!(num = light_cones.len(), "found light cones");
        self.light_cones.append(&mut light_cones);
    }

    pub fn add_characters(&mut self, characters: GetAvatarDataScRsp) {
        let (characters, multipath_characters) =
            characters.avatar_list.iter().partition::<Vec<_>, _>(|a| {
                MultiPathAvatarType::from_i32(a.base_avatar_id as i32).is_none()
            });

        let mut characters: Vec<Character> = characters
            .iter()
            .filter_map(|char| export_proto_character(&self.database, char))
            .collect();

        info!(num = characters.len(), "found characters");
        self.characters.append(&mut characters);

        info!(
            num = multipath_characters.len(),
            "found multipath base avatars"
        );
        self.multipath_base_avatars.extend(
            multipath_characters
                .into_iter()
                .map(|c| (c.base_avatar_id, c.clone())),
        );
    }

    pub fn add_multipath_characters(&mut self, characters: GetMultiPathAvatarInfoScRsp) {
        let mut characters: Vec<Character> = characters
            .multi_path_avatar_type_info_list
            .iter()
            .filter_map(|char| export_proto_multipath_character(&self.database, char))
            .collect();

        // Try to find a trailblazer to determine the gender
        if let Some(trailblazer) = characters.iter().find(|c| c.name == "Trailblazer") {
            self.trailblazer = Some(if trailblazer.id.parse::<u32>().unwrap() % 2 == 0 {
                "Stelle"
            } else {
                "Caelus"
            });
        }

        info!(num = characters.len(), "found multipath characters");
        self.multipath_characters.append(&mut characters);
    }

    pub fn finalize_multipath_characters(&mut self) {
        // Fetch level & ascension
        for character in self.multipath_characters.iter_mut() {
            if let Some(config) = self
                .database
                .multipath_avatar_config
                .get(&character.id.parse().unwrap())
            {
                if let Some(base_avatar) = self.multipath_base_avatars.get(&config.BaseAvatarID) {
                    character.level = base_avatar.level;
                    character.ascension = base_avatar.promotion;
                }
            }
        }
    }
}

impl Exporter for OptimizerExporter {
    type Export = Export;

    fn read_command(&mut self, command: GameCommand) {
        match command.command_id {
            command_id::PlayerGetTokenScRsp => {
                debug!("detected uid");
                let cmd = command.parse_proto::<PlayerGetTokenScRsp>();
                match cmd {
                    Ok(cmd) => self.set_uid(cmd.uid),
                    Err(error) => {
                        warn!(%error, "could not parse token command");
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
            _ => {
                trace!(
                    command_id = command.command_id,
                    tag = command.get_command_name(),
                    "ignored"
                );
            }
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
        self.trailblazer.is_some()
            && self.uid.is_some()
            && !self.relics.is_empty()
            && !self.characters.is_empty()
            && !self.multipath_characters.is_empty()
            && !self.light_cones.is_empty()
    }

    #[instrument(skip_all)]
    fn export(mut self) -> Option<Self::Export> {
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
        } else {
            self.finalize_multipath_characters();
        }

        let export = Export {
            source: "reliquary_archiver",
            build: env!("CARGO_PKG_VERSION"),
            version: 4,
            metadata: Metadata {
                uid: self.uid,
                trailblazer: self.trailblazer,
            },
            light_cones: self.light_cones,
            relics: self.relics,
            characters: self
                .characters
                .into_iter()
                .chain(self.multipath_characters)
                .collect(),
        };

        Some(export)
    }
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

    let id = proto.unique_id.to_string();
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
        set_id: set_id.to_string(),
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

#[derive(Serialize, Deserialize, Debug)]
pub struct Relic {
    pub set_id: String,
    pub name: String,
    pub slot: &'static str,
    pub rarity: u32,
    pub level: u32,
    pub mainstat: String,
    pub substats: Vec<Substat>,
    pub location: String,
    pub lock: bool,
    pub discard: bool,
    pub _uid: String,
}

#[derive(Serialize, Deserialize, Debug)]
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

#[derive(Serialize, Deserialize, Debug)]
pub struct LightCone {
    pub id: String,
    pub name: String,
    pub level: u32,
    pub ascension: u32,
    pub superimposition: u32,
    pub location: String,
    pub lock: bool,
    pub _uid: String,
}

#[instrument(name = "light_cone", skip_all, fields(id = proto.tid))]
fn export_proto_light_cone(db: &Database, proto: &ProtoLightCone) -> Option<LightCone> {
    let cfg = db.equipment_config.get(&proto.tid)?;
    let id = cfg.EquipmentID.to_string();
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
        _uid: proto.unique_id.to_string(),
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
        id: id.to_string(),
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

    // TODO: figure out where level/ascension is stored
    Some(Character {
        id: id.to_string(),
        name,
        path,
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

#[derive(Serialize, Deserialize, Debug)]
pub struct Character {
    pub id: String,
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

#[derive(Serialize, Deserialize, Debug)]
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
