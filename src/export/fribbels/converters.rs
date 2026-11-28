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

/// A relic's max level is this many times its rarity
const MAX_LEVEL_PER_RARITY: u32 = 3;

/// The fields every relic has, from its config and main stat. Logs why a relic is left out.
fn export_relic_base(db: &Database, tid: u32, main_affix_id: u32, level: u32, sub_affixes: &[RelicAffix]) -> Option<BoxRelic> {
    let Some(relic_config) = db.relic_config.get(&tid) else {
        warn!("unknown relic, skipping");
        return None;
    };

    let set_id = relic_config.SetID;
    let Some(set_config) = db.relic_set_config.get(&set_id) else {
        warn!(set_id, "unknown relic set, skipping relic");
        return None;
    };
    let Some(main_affix_config) = db.relic_main_affix_config.get(&relic_config.MainAffixGroup, &main_affix_id) else {
        warn!(main_affix_id, "unknown main affix, skipping relic");
        return None;
    };

    let name = set_config.SetName.lookup(&db.text_map).map(|s| s.to_string()).unwrap_or_default();
    let slot = slot_type_to_export(&relic_config.Type)?;
    let rarity = relic_config.MaxLevel / MAX_LEVEL_PER_RARITY;
    let mainstat = main_stat_to_export(&main_affix_config.Property)?.to_string();

    Some(BoxRelic {
        set_id,
        name,
        slot,
        rarity,
        level,
        mainstat,
        substats: export_substats(db, rarity, sub_affixes),
    })
}

fn export_substats(db: &Database, rarity: u32, affixes: &[RelicAffix]) -> Vec<Substat> {
    affixes.iter().filter_map(|substat| export_substat(db, rarity, substat)).collect()
}

/// Converts a proto relic to an export relic
#[instrument(name = "relic", skip_all, fields(id = proto.tid))]
pub fn export_proto_relic(db: &Database, proto: &ProtoRelic) -> Option<Relic> {
    let BoxRelic {
        set_id,
        name,
        slot,
        rarity,
        level,
        mainstat,
        substats,
    } = export_relic_base(db, proto.tid, proto.main_affix_id, proto.level, &proto.sub_affix_list)?;

    let location = format_location(proto.equip_avatar_id);

    debug!(rarity, set_name = name, slot, mainstat, location, "detected");

    let reroll_substats = (!proto.reforge_sub_affix_list.is_empty()).then(|| export_substats(db, rarity, &proto.reforge_sub_affix_list));
    let preview_substats = (!proto.preview_sub_affix_list.is_empty()).then(|| export_substats(db, rarity, &proto.preview_sub_affix_list));

    Some(Relic {
        set_id,
        name,
        slot,
        rarity,
        level,
        mainstat,
        substats,
        reroll_substats,
        preview_substats,
        location,
        lock: proto.is_protected,
        discard: proto.is_discarded,
        _uid: proto.unique_id,
    })
}

/// Converts a relic that relic recon offers to choose
#[instrument(name = "relic", skip_all, fields(id = proto.tid))]
pub fn export_proto_box_relic(db: &Database, proto: &ProtoBoxRelic) -> Option<BoxRelic> {
    export_relic_base(db, proto.tid, proto.main_affix_id, proto.level, &proto.sub_affix_list)
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
        trace!(point = skill.point_id, level, "detected skill tree point");

        match skill.point_id {
            1 => skills.basic = level,
            2 => skills.skill = level,
            3 => skills.ult = level,
            4 => skills.talent = level,
            // The technique isn't exported
            5 => {}
            6 => traces.ability_1 = true,
            7 => traces.ability_2 = true,
            8 => traces.ability_3 = true,
            9 => traces.stat_1 = true,
            10 => traces.stat_2 = true,
            11 => traces.stat_3 = true,
            12 => traces.stat_4 = true,
            13 => traces.stat_5 = true,
            14 => traces.stat_6 = true,
            15 => traces.stat_7 = true,
            16 => traces.stat_8 = true,
            17 => traces.stat_9 = true,
            18 => traces.stat_10 = true,
            19 => memosprite.skill = level,
            20 => memosprite.talent = level,
            21 => traces.special = true,
            22 => skills.elation = Some(level),
            // Aeon Aha's elation debt
            23 => traces.elation_debt = true,
            _ => warn!(anchor = skill.point_id, "unknown point anchor"),
        }
    }

    (skills, traces, memosprite.if_present())
}
