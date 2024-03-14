use reliquary::network::gen::proto::Relic::Relic as ProtoRelic;
use serde::{Deserialize, Serialize};

use crate::export::optimizer::ConfigMaps;

pub fn export_proto_relic(config: &ConfigMaps, proto: &ProtoRelic) -> Relic {
    let relic_config = config.relic_config.get(&proto.tid).unwrap();
    let set_config = config.relic_set_config.get(&relic_config.SetID).unwrap();
    let main_affix_config = config.relic_main_affix_config.get(&relic_config.MainAffixGroup, &proto.main_affix_id).unwrap();

    let set = set_config.SetName.lookup(&config.text_map).unwrap().to_string();
    let slot = slot_type_to_export(&relic_config.Type).to_string();
    let rarity = relic_config.MaxLevel / 3;
    let mainstat = main_stat_to_export(&main_affix_config.Property).to_string();

    let substats = proto.sub_affix_list.iter()
        .map(|substat| {
            let substat_config = config.relic_sub_affix_config.get(&rarity, &substat.affix_id).unwrap();
            let key = sub_stat_to_export(&substat_config.Property).to_string();
            let mut value = substat.cnt as f32 * *substat_config.BaseValue
                + substat.step as f32 * *substat_config.StepValue;

            if key.ends_with('_') {
                value *= 100.0;
            }

            Substat {
                key,
                value,
            }
        })
        .collect();

    let location = if proto.base_avatar_id != 0 {
        config.avatar_config
            .get(&proto.base_avatar_id).unwrap()
            .AvatarName
            .lookup(&config.text_map).unwrap()
            .to_string()
    } else {
        "".to_string()
    };

    Relic {
        set,
        slot,
        rarity,
        level: proto.level,
        mainstat,
        substats,
        location,
        lock: proto.is_protected,
        discard: proto.is_discarded,
        _id: proto.unique_id.to_string(),
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Relic {
    pub set: String,
    pub slot: String,
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
        _ => panic!("Unknown sub stat: {}", s)
    }
}