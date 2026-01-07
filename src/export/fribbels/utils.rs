use tracing::debug;

/// Formats character ID as location string for relics and light cones
pub fn format_location(avatar_id: u32) -> String {
    if avatar_id == 0 {
        "".to_owned()
    } else {
        avatar_id.to_string()
    }
}

/// Converts slot type from game format to export format
pub fn slot_type_to_export(s: &str) -> &'static str {
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

/// Converts main stat property from game format to export format
pub fn main_stat_to_export(s: &str) -> &'static str {
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

/// Converts sub stat property from game format to export format
pub fn sub_stat_to_export(s: &str) -> &'static str {
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

/// Converts avatar base type to path for characters
pub fn avatar_path_lookup(db: &crate::export::database::Database, avatar_id: u32) -> Option<&'static str> {
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
