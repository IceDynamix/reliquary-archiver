use std::sync::OnceLock;

use reliquary::resource::excel::{
    AvatarConfigMap, AvatarExRankConfigMap, AvatarSkillTreeConfigMap, EquipmentConfigMap, ItemConfigMap, MultiplePathAvatarConfigMap,
    RelicConfigMap, RelicMainAffixConfigMap, RelicSetConfigMap, RelicSubAffixConfigMap,
};
use reliquary::resource::text_map::TextMap;
use tracing::instrument;

pub struct Database {
    pub avatar_config: AvatarConfigMap,
    pub avatar_ex_rank_config: AvatarExRankConfigMap,
    pub avatar_skill_tree_config: AvatarSkillTreeConfigMap,
    pub equipment_config: EquipmentConfigMap,
    pub item_config: ItemConfigMap,
    pub multipath_avatar_config: MultiplePathAvatarConfigMap,
    pub relic_config: RelicConfigMap,
    pub relic_set_config: RelicSetConfigMap,
    pub relic_main_affix_config: RelicMainAffixConfigMap,
    pub relic_sub_affix_config: RelicSubAffixConfigMap,
    pub text_map: TextMap,
}

pub fn get_database() -> &'static Database {
    static DATABASE: OnceLock<Database> = OnceLock::new();
    DATABASE.get_or_init(Database::new)
}

impl Default for Database {
    fn default() -> Self {
        Self::new()
    }
}

/// Parses a config file that the build script wrote to OUT_DIR
macro_rules! config {
    ($file:literal) => {
        serde_json::from_str(include_str!(concat!(env!("OUT_DIR"), "/", $file))).unwrap()
    };
}

impl Database {
    /// Trailblazer avatar ids start here, one for each path and gender
    const FIRST_TRAILBLAZER_ID: u32 = 8000;

    #[instrument(name = "config_map")]
    pub fn new() -> Self {
        Database {
            avatar_config: config!("AvatarConfig.json"),
            avatar_ex_rank_config: config!("AvatarExRankConfig.json"),
            avatar_skill_tree_config: config!("AvatarSkillTreeConfig.json"),
            equipment_config: config!("EquipmentConfig.json"),
            item_config: config!("ItemConfig.json"),
            multipath_avatar_config: config!("MultiplePathAvatarConfig.json"),
            relic_config: config!("RelicConfig.json"),
            relic_set_config: config!("RelicSetConfig.json"),
            relic_main_affix_config: config!("RelicMainAffixConfig.json"),
            relic_sub_affix_config: config!("RelicSubAffixConfig.json"),
            text_map: config!("TextMapEN.json"),
        }
    }

    /// The English name of a character or light cone that can be pulled from a warp
    pub fn warp_item_name(&self, item_id: u32) -> Option<String> {
        let name = match self.equipment_config.get(&item_id) {
            Some(cfg) => cfg.EquipmentName.lookup(&self.text_map).map(|s| s.to_string()),
            None => self.lookup_avatar_name(item_id),
        }?;
        // Names carry the game's markup, like "Silver Wolf LV.<unbreak>999</unbreak>"
        let mut plain = String::with_capacity(name.len());
        let mut in_tag = false;
        for c in name.chars() {
            match c {
                '<' => in_tag = true,
                '>' if in_tag => in_tag = false,
                c if !in_tag => plain.push(c),
                _ => {}
            }
        }
        Some(plain)
    }

    /// The star rating of a character or light cone that can be pulled from a warp
    pub fn warp_item_rarity(&self, item_id: u32) -> Option<u8> {
        let rarity = match self.equipment_config.get(&item_id) {
            Some(cfg) => cfg.Rarity.as_str(),
            None => self.avatar_config.get(&item_id)?.Rarity.as_str(),
        };
        // "CombatPowerLightconeRarity5", "CombatPowerAvatarRarityType4" and so on
        let stars = rarity.rsplit_once("Rarity")?.1.trim_start_matches("Type");
        stars.parse().ok().filter(|stars| (1..=5).contains(stars))
    }

    /// Whether the item is a light cone rather than a character
    pub fn is_light_cone(&self, item_id: u32) -> bool {
        self.equipment_config.get(&item_id).is_some()
    }

    /// Whether the avatar is one of the trailblazers
    pub fn is_trailblazer(avatar_id: u32) -> bool {
        avatar_id >= Self::FIRST_TRAILBLAZER_ID
    }

    pub(crate) fn lookup_avatar_name(&self, avatar_id: u32) -> Option<String> {
        if avatar_id == 0 {
            return None;
        }

        if Self::is_trailblazer(avatar_id) {
            Some("Trailblazer".to_owned())
        } else {
            let cfg = self.avatar_config.get(&avatar_id)?;
            cfg.AvatarName.lookup(&self.text_map).map(|s| s.to_string())
        }
    }
}
