use reliquary::network::command::proto::AvatarPathData::AvatarPathData as ProtoAvatarPathData;
use reliquary::network::command::proto::AvatarPathSkillTree::AvatarPathSkillTree as ProtoSkillTree;
use reliquary::network::command::proto::BoxRelic::BoxRelic as ProtoBoxRelic;
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
    let Some(relic_config) = db.relic_config.get(&proto.tid) else {
        warn!("unknown relic, skipping");
        return None;
    };

    let set_id = relic_config.SetID;
    let Some(set_config) = db.relic_set_config.get(&set_id) else {
        warn!(set_id, "unknown relic set, skipping relic");
        return None;
    };
    let Some(main_affix_config) = db.relic_main_affix_config.get(&relic_config.MainAffixGroup, &proto.main_affix_id) else {
        warn!(main_affix_id = proto.main_affix_id, "unknown main affix, skipping relic");
        return None;
    };

    let id = proto.unique_id;
    let level = proto.level;
    let lock = proto.is_protected;
    let discard = proto.is_discarded;
    let set_name = set_config
        .SetName
        .lookup(&db.text_map)
        .map(|s| s.to_string())
        .unwrap_or("".to_string());

    let slot = slot_type_to_export(&relic_config.Type)?;
    let rarity = relic_config.MaxLevel / 3;
    let mainstat = main_stat_to_export(&main_affix_config.Property)?.to_string();
    let location = format_location(proto.equip_avatar_id);

    debug!(rarity, set_name, slot, mainstat, location, "detected");

    let export_substats =
        |affixes: &[RelicAffix]| -> Vec<Substat> { affixes.iter().filter_map(|substat| export_substat(db, rarity, substat)).collect() };

    let substats = export_substats(&proto.sub_affix_list);
    let reroll_substats = (!proto.reforge_sub_affix_list.is_empty()).then(|| export_substats(&proto.reforge_sub_affix_list));
    let preview_substats = (!proto.preview_sub_affix_list.is_empty()).then(|| export_substats(&proto.preview_sub_affix_list));

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

#[tracing::instrument(name = "relic", skip_all, fields(id = proto.tid))]
pub fn export_proto_box_relic(db: &Database, proto: &ProtoBoxRelic) -> Option<BoxRelic> {
    let Some(relic_config) = db.relic_config.get(&proto.tid) else {
        warn!("unknown relic, skipping");
        return None;
    };

    let set_id = relic_config.SetID;
    let Some(set_config) = db.relic_set_config.get(&set_id) else {
        warn!(set_id, "unknown relic set, skipping relic");
        return None;
    };
    let Some(main_affix_config) = db.relic_main_affix_config.get(&relic_config.MainAffixGroup, &proto.main_affix_id) else {
        warn!(main_affix_id = proto.main_affix_id, "unknown main affix, skipping relic");
        return None;
    };

    let slot = slot_type_to_export(&relic_config.Type)?;
    let rarity = relic_config.MaxLevel / 3;

    let export_substats =
        |affixes: &[RelicAffix]| -> Vec<Substat> { affixes.iter().filter_map(|substat| export_substat(db, rarity, substat)).collect() };

    let mainstat = main_stat_to_export(&main_affix_config.Property)?.to_string();
    let substats = export_substats(&proto.sub_affix_list);

    let set_name = set_config
        .SetName
        .lookup(&db.text_map)
        .map(|s| s.to_string())
        .unwrap_or("".to_string());

    Some(BoxRelic {
        set_id,
        name: set_name,
        slot,
        rarity,
        level: proto.level,
        mainstat,
        substats,
    })
}

/// Converts a relic substat to an export substat
#[tracing::instrument(name = "substat", skip_all)]
pub fn export_substat(db: &Database, rarity: u32, substat: &RelicAffix) -> Option<Substat> {
    let Some(cfg) = db.relic_sub_affix_config.get(&rarity, &substat.affix_id) else {
        // The relic is still exported, just without this substat, so this one matters most
        warn!(rarity, affix_id = substat.affix_id, "unknown substat, leaving it out of the relic");
        return None;
    };
    let key = sub_stat_to_export(&cfg.Property)?.to_string();

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
    let Some(cfg) = db.equipment_config.get(&proto.tid) else {
        warn!("unknown light cone, skipping");
        return None;
    };
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

/// Converts a proto multipath character to an export character
pub fn export_proto_multipath_character(db: &Database, proto: &ProtoAvatarPathData) -> Option<Character> {
    let id = proto.avatar_id;
    let name = db.lookup_avatar_name(id)?;
    let path = avatar_path_lookup(db, id)?.to_owned();
    let affinitive_eidolons = db
        .avatar_ex_rank_config
        .0
        .iter()
        .any(|config| config.ACCJKGEKHKP == id)
        .then(|| proto.ExRankList.clone());

    let span = info_span!("character", name, path);
    let _enter = span.enter();

    trace!(character = name, path, "detected");

    let (skills, traces, memosprite) = export_skill_tree(&proto.avatar_path_skill_tree);
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
        affinitive_eidolons,
        ability_version,
    })
}

/// Extracts skills, traces, and memosprite from a skill tree
pub fn export_skill_tree(proto: &[ProtoSkillTree]) -> (Skills, Traces, Option<Memosprite>) {
    let mut skills = Skills::default();
    let mut traces = Traces::default();
    let mut memosprite = Memosprite::default();

    for skill in proto.iter().filter(|s| s.point_id != 0) {
        let level = skill.level;

        let span = info_span!("skill", id = skill.point_id, level);
        let _enter = span.enter();

        match skill.point_id {
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
            22 => {
                trace!("detected elation skill trace");
                skills.elation = Some(level);
            }
            23 => {
                trace!("detected Aeon Aha elation debt trace");
                traces.elation_debt = true;
            }

            _ => {
                warn!(anchor = skill.point_id, "unknown point anchor");
                continue;
            }
        }
    }

    (skills, traces, memosprite.if_present())
}
