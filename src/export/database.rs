use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use reliquary::resource::excel::{
    AvatarConfigMap, AvatarSkillTreeConfigMap, EquipmentConfigMap, ItemConfigMap, MultiplePathAvatarConfigMap,
    RelicConfigMap, RelicMainAffixConfigMap, RelicSetConfigMap, RelicSubAffixConfigMap,
};
use reliquary::resource::text_map::TextMap;
use serde::de::DeserializeOwned;
use std::collections::HashMap;
use tracing::{info, instrument};

pub struct Database {
    pub avatar_config: AvatarConfigMap,
    pub avatar_skill_tree_config: AvatarSkillTreeConfigMap,
    pub equipment_config: EquipmentConfigMap,
    pub item_config: ItemConfigMap,
    pub multipath_avatar_config: MultiplePathAvatarConfigMap,
    pub relic_config: RelicConfigMap,
    pub relic_set_config: RelicSetConfigMap,
    pub relic_main_affix_config: RelicMainAffixConfigMap,
    pub relic_sub_affix_config: RelicSubAffixConfigMap,
    pub text_map: TextMap,
    pub keys: HashMap<u32, Vec<u8>>,
}

impl Database {
    #[instrument(name = "config_map")]
    pub fn new() -> Self {
        info!("using local database");

        // config files are downloaded by the build script
        //
        // i *would* create a fn load_local_config<T: ResourceMap + DeserializeOwned>()
        // to avoid duplicating the json file names by using T::get_json_name,
        // but concat!() only takes string literals. it doesn't even take `&'static str`!!
        // https://github.com/rust-lang/rust/issues/53749
        Database {
            avatar_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/AvatarConfig.json"
            ))),
            avatar_skill_tree_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/AvatarSkillTreeConfig.json"
            ))),
            equipment_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/EquipmentConfig.json"
            ))),
            item_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/ItemConfig.json"
            ))),
            multipath_avatar_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/MultiplePathAvatarConfig.json"
            ))),
            relic_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/RelicConfig.json"
            ))),
            relic_set_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/RelicSetConfig.json"
            ))),
            relic_main_affix_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/RelicMainAffixConfig.json"
            ))),
            relic_sub_affix_config: Self::parse_json(include_str!(concat!(
                env!("OUT_DIR"),
                "/RelicSubAffixConfig.json"
            ))),
            text_map: Self::parse_json(include_str!(concat!(env!("OUT_DIR"), "/TextMapEN.json"))),
            keys: Self::load_local_keys(),
        }
    }

    fn parse_json<T: DeserializeOwned>(str: &'static str) -> T {
        serde_json::de::from_str(str).unwrap()
    }

    fn load_local_keys() -> HashMap<u32, Vec<u8>> {
        let keys: HashMap<u32, String> =
            Self::parse_json(include_str!(concat!(env!("OUT_DIR"), "/keys.json")));
        let mut keys_bytes = HashMap::new();

        for (k, v) in keys {
            keys_bytes.insert(k, BASE64_STANDARD.decode(v).unwrap());
        }

        keys_bytes
    }

    pub(crate) fn lookup_avatar_name(&self, avatar_id: u32) -> Option<String> {
        if avatar_id == 0 {
            return None;
        }

        if avatar_id >= 8000 {
            Some("Trailblazer".to_owned())
        } else {
            let cfg = self.avatar_config.get(&avatar_id)?;
            cfg.AvatarName.lookup(&self.text_map).map(|s| s.to_string())
        }
    }
}
