use reliquary::network::gen::proto::Gender::Gender;
use reliquary::network::gen::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::gen::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::gen::proto::GetHeroBasicTypeInfoScRsp::GetHeroBasicTypeInfoScRsp;
use reliquary::resource::excel::*;
use reliquary::resource::ResourceMap;
use reliquary::resource::text_map::TextMap;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use tracing::info;

use crate::export::optimizer::character::{Character, export_proto_character};
use crate::export::optimizer::light_cone::{export_proto_light_cone, LightCone};
use crate::export::optimizer::relic::export_proto_relic;
use crate::export::optimizer::relic::Relic;

mod relic;
mod character;
mod light_cone;

const BASE_RESOURCE_URL: &str = "https://raw.githubusercontent.com/Dimbreath/StarRailData/master";


// TODO: encryption key map

struct ConfigMaps {
    avatar_config: AvatarConfigMap,
    avatar_skill_tree_config: AvatarSkillTreeConfigMap,
    equipment_config: EquipmentConfigMap,
    relic_config: RelicConfigMap,
    relic_set_config: RelicSetConfigMap,
    relic_main_affix_config: RelicMainAffixConfigMap,
    relic_sub_affix_config: RelicSubAffixConfigMap,
    text_map: TextMap,
}

impl ConfigMaps {
    pub fn new_from_online() -> Self {
        ConfigMaps {
            avatar_config: Self::load_config(),
            avatar_skill_tree_config: Self::load_config(),
            equipment_config: Self::load_config(),
            relic_config: Self::load_config(),
            relic_set_config: Self::load_config(),
            relic_main_affix_config: Self::load_config(),
            relic_sub_affix_config: Self::load_config(),
            text_map: Self::load_text_map(),
        }
    }

    fn load_config<T: ResourceMap + DeserializeOwned>() -> T {
        Self::get::<T>(format!("{BASE_RESOURCE_URL}/ExcelOutput/{}", T::get_json_name()))
    }

    fn load_text_map() -> TextMap {
        Self::get(format!("{BASE_RESOURCE_URL}/TextMap/TextMapEN.json"))
    }

    fn get<T: DeserializeOwned>(url: String) -> T {
        info!("requesting {}", url);
        ureq::get(&url)
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }
}

pub struct ExportForOptimizer {
    config: ConfigMaps,
    output: Export,
}

impl ExportForOptimizer {
    pub fn new_from_online() -> ExportForOptimizer {
        ExportForOptimizer {
            config: ConfigMaps::new_from_online(),
            output: Export {
                source: "reliquary-archiver".to_string(),
                build: "v0.1.0".to_string(),
                version: 3,
                metadata: Metadata {
                    uid: None,
                    trailblazer: None,
                },
                light_cones: vec![],
                relics: vec![],
                characters: vec![],
            },
        }
    }

    pub fn write_uid(&mut self, uid: String) {
        self.output.metadata.uid = Some(uid.parse::<u32>().unwrap());
    }

    pub fn write_hero(&mut self, hero: GetHeroBasicTypeInfoScRsp) {
        let trailblazer_title = match hero.gender.enum_value().unwrap() {
            Gender::GenderNone => "Pompom", // lmao
            Gender::GenderMan => "Caelus",
            Gender::GenderWoman => "Stelle"
        };

        // TODO: handle trailblazer build

        self.output.metadata.trailblazer = Some(trailblazer_title.to_string());
    }

    pub fn write_bag(&mut self, bag: GetBagScRsp) {
        self.output.relics = bag.relic_list.iter()
            .map(|r| export_proto_relic(&self.config, r))
            .collect();

        self.output.light_cones = bag.equipment_list.iter()
            .map(|equip| export_proto_light_cone(&self.config, equip))
            .collect();
    }

    pub fn write_characters(&mut self, characters: GetAvatarDataScRsp) {
        self.output.characters = characters.avatar_list.iter()
            .filter(|a| a.base_avatar_id < 8000) // skip trailblazer, handled in `write_hero`
            .map(|char| export_proto_character(&self.config, char))
            .collect();
    }

    pub fn export(&self) -> &Export {
        &self.output
    }
}


#[derive(Serialize, Deserialize, Debug)]
pub struct Export {
    pub source: String,
    pub build: String,
    pub version: u32,
    pub metadata: Metadata,
    pub light_cones: Vec<LightCone>,
    pub relics: Vec<Relic>,
    pub characters: Vec<Character>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Metadata {
    pub uid: Option<u32>,
    pub trailblazer: Option<String>,
}