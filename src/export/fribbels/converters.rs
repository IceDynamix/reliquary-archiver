use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::proto::AvatarPathData::AvatarPathData as ProtoAvatarPathData;
use reliquary::network::command::proto::AvatarPathSkillTree::AvatarPathSkillTree as ProtoSkillTree;
use reliquary::network::command::proto::Equipment::Equipment as ProtoLightCone;
use reliquary::network::command::proto::Material::Material as ProtoMaterial;
use reliquary::network::command::proto::Relic::Relic as ProtoRelic;
use reliquary::network::command::proto::RelicAffix::RelicAffix;
use tracing::{debug, info_span, instrument, trace, warn};

use crate::export::database::Database;
use crate::export::fribbels::models::*;
use crate::export::fribbels::utils::*;

/// Converts a proto material to an export material
#[instrument(name = "material", skip_all, fields(id = proto.tid))]
pub fn export_proto_material(db: &Database, proto: &ProtoMaterial) -> Option<Material> {
    let cfg = db.item_config.get(&proto.tid)?;
    let id = cfg.ID;
    let name = cfg.ItemName.and_then(|hash| db.text_map.get(&hash)).map(|s| s.to_string())?;
    let count = proto.num;

    debug!(material = name, count, "detected");

    Some(Material {
        id,
        name,
        count,
        expire_time: None,
    })
}

/// Converts a proto relic to an export relic
#[tracing::instrument(name = "relic", skip_all, fields(id = proto.tid))]
pub fn export_proto_relic(db: &Database, proto: &ProtoRelic) -> Option<Relic> {
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

    let reroll_substats = if proto.reforge_sub_affix_list.is_empty() {
        None
    } else {
        Some(
            proto
                .reforge_sub_affix_list
                .iter()
                .filter_map(|substat| export_substat(db, rarity, substat))
                .collect(),
        )
    };

    let preview_substats = if proto.preview_sub_affix_list.is_empty() {
        None
    } else {
        Some(
            proto
                .preview_sub_affix_list
                .iter()
                .filter_map(|substat| export_substat(db, rarity, substat))
                .collect(),
        )
    };

    Some(Relic {
        set_id,
        name: set_name,
        slot,
        rarity,
        level,
        mainstat,
        substats,
        reroll_substats,
        preview_substats,
        location,
        lock,
        discard,
        _uid: id,
    })
}

/// Converts a relic substat to an export substat
#[tracing::instrument(name = "substat", skip_all)]
pub fn export_substat(db: &Database, rarity: u32, substat: &RelicAffix) -> Option<Substat> {
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

/// Converts a proto light cone to an export light cone
#[instrument(name = "light_cone", skip_all, fields(id = proto.tid))]
pub fn export_proto_light_cone(db: &Database, proto: &ProtoLightCone) -> Option<LightCone> {
    let cfg = db.equipment_config.get(&proto.tid)?;
    let id = cfg.EquipmentID;
    let name = cfg.EquipmentName.lookup(&db.text_map).map(|s| s.to_string())?;

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

/// Converts a proto character to an export character
#[instrument(name = "character", skip_all, fields(id = proto.base_avatar_id))]
pub fn export_proto_character(db: &Database, proto: &ProtoCharacter) -> Option<Character> {
    let id = proto.base_avatar_id;
    let name = db.lookup_avatar_name(id)?;
    let path = avatar_path_lookup(db, id)?.to_owned();

    let level = proto.level;
    // let eidolon = proto.rank;

    debug!(character = name, level, "detected");

    // let (skills, traces, memosprite) = export_skill_tree(db, &proto.);
    // let ability_version = proto.skilltree_version;

    Some(Character {
        id,
        name,
        path,
        level,
        ascension: proto.promotion,

        // These fields are only accessible in the AvatarPathData
        // Will be updated in [resolve_multipath_character]
        eidolon: 0,
        skills: Skills::default(),
        traces: Traces::default(),
        memosprite: None,
        ability_version: 0,
    })
}

/// Converts a proto multipath character to an export character
pub fn export_proto_multipath_character(db: &Database, proto: &ProtoAvatarPathData) -> Option<Character> {
    let id = proto.avatar_id;
    let name = db.lookup_avatar_name(id)?;
    let path = avatar_path_lookup(db, id)?.to_owned();

    let span = info_span!("character", name, path);
    let _enter = span.enter();

    trace!(character = name, path, "detected");

    let (skills, traces, memosprite) = export_skill_tree(db, &proto.avatar_path_skill_tree);
    let ability_version = proto.skilltree_version;

    Some(Character {
        id,
        name,
        path,
        // Level and ascension are stored in the base avatar
        // in the main character list, set them to 0 for now.
        // Will be updated in [resolve_multipath_character]
        level: 0,
        ascension: 0,
        eidolon: proto.rank,
        skills,
        traces,
        memosprite,
        ability_version,
    })
}

/// Extracts skills, traces, and memosprite from a skill tree
pub fn export_skill_tree(_db: &Database, proto: &[ProtoSkillTree]) -> (Skills, Traces, Option<Memosprite>) {
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
        special: false,
    };

    let mut memosprite = Memosprite { skill: 0, talent: 0 };

    for skill in proto.iter().filter(|s| s.multi_point_id != 0) {
        let level = skill.level;

        let span = info_span!("skill", id = skill.multi_point_id, level);
        let _enter = span.enter();

        match skill.multi_point_id {
            1 => {
                trace!(level, "detected basic atk trace");
                skills.basic = level;
            }
            2 => {
                trace!(level, "detected skill trace");
                skills.skill = level;
            }
            3 => {
                trace!(level, "detected ult trace");
                skills.ult = level;
            }
            4 => {
                trace!(level, "detected talent trace");
                skills.talent = level;
            }

            5 => {
                trace!(level, "detected technique trace");
                /* technique */
            }

            6 => {
                trace!("detected major trace 1");
                traces.ability_1 = true;
            }
            7 => {
                trace!("detected major trace 2");
                traces.ability_2 = true;
            }
            8 => {
                trace!("detected major trace 3");
                traces.ability_3 = true;
            }

            9 => {
                trace!("detected minor trace 1");
                traces.stat_1 = true;
            }
            10 => {
                trace!("detected minor trace 2");
                traces.stat_2 = true;
            }
            11 => {
                trace!("detected minor trace 3");
                traces.stat_3 = true;
            }
            12 => {
                trace!("detected minor trace 4");
                traces.stat_4 = true;
            }
            13 => {
                trace!("detected minor trace 5");
                traces.stat_5 = true;
            }
            14 => {
                trace!("detected minor trace 6");
                traces.stat_6 = true;
            }
            15 => {
                trace!("detected minor trace 7");
                traces.stat_7 = true;
            }
            16 => {
                trace!("detected minor trace 8");
                traces.stat_8 = true;
            }
            17 => {
                trace!("detected minor trace 9");
                traces.stat_9 = true;
            }
            18 => {
                trace!("detected minor trace 10");
                traces.stat_10 = true;
            }
            19 => {
                trace!("detected memosprite skill trace");
                memosprite.skill = level;
            }
            20 => {
                trace!("detected memosprite talent trace");
                memosprite.talent = level;
            }
            21 => {
                trace!("detected special trace",);
                traces.special = true;
            }

            _ => {
                warn!(anchor = skill.multi_point_id, "unknown point anchor");
                continue;
            }
        }
    }

    (skills, traces, memosprite.if_present())
}
