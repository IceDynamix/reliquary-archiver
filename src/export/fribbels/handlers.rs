use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::proto::AvatarPathData::AvatarPathData;
use reliquary::network::command::proto::DoGachaScRsp::DoGachaScRsp;
use reliquary::network::command::proto::Equipment::Equipment as ProtoLightCone;
use reliquary::network::command::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::command::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::command::proto::GetGachaInfoScRsp::GetGachaInfoScRsp;
use reliquary::network::command::proto::Material::Material as ProtoMaterial;
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use reliquary::network::command::proto::PlayerLoginScRsp::PlayerLoginScRsp;
use reliquary::network::command::proto::PlayerSyncScNotify::PlayerSyncScNotify;
use reliquary::network::command::proto::Relic::Relic as ProtoRelic;
use reliquary::network::command::proto::SetAvatarEnhancedIdScRsp::SetAvatarEnhancedIdScRsp;
use tracing::{debug, info, warn};

use super::OptimizerExporter;
use crate::export::fribbels::converters::*;
use crate::export::fribbels::models::*;

impl OptimizerExporter {
    pub fn handle_token(&mut self, token: PlayerGetTokenScRsp) {
        self.uid = Some(token.uid);
    }

    pub fn handle_login(&mut self, login: PlayerLoginScRsp) {
        self.gacha.oneric_shards = login.basic_info.oneric_shard_count;
        self.gacha.stellar_jade = login.basic_info.stellar_jade_count;
    }

    /// Converts the items and stores them, returning what was stored
    fn ingest_items(
        &mut self,
        relics: &[ProtoRelic],
        light_cones: &[ProtoLightCone],
        materials: &[ProtoMaterial],
    ) -> (Vec<Relic>, Vec<LightCone>, Vec<Material>) {
        let relics: Vec<Relic> = relics.iter().filter_map(|r| export_proto_relic(self.database, r)).collect();
        for relic in &relics {
            self.relics.insert(relic._uid, relic.clone());
        }

        let light_cones: Vec<LightCone> = light_cones
            .iter()
            .filter_map(|equip| export_proto_light_cone(self.database, equip))
            .collect();
        for light_cone in &light_cones {
            self.light_cones.insert(light_cone._uid, light_cone.clone());
        }

        let materials: Vec<Material> = materials.iter().filter_map(|m| export_proto_material(self.database, m)).collect();
        for material in &materials {
            self.materials.insert(material.id, material.clone());
        }

        (relics, light_cones, materials)
    }

    pub fn handle_inventory(&mut self, bag: GetBagScRsp) {
        let (relics, light_cones, materials) = self.ingest_items(&bag.relic_list, &bag.equipment_list, &bag.material_list);

        info!(num = relics.len(), "found relics");
        info!(num = light_cones.len(), "found light cones");
        info!(num = materials.len(), "found materials");
    }

    pub fn ingest_character(&mut self, proto_character: &ProtoCharacter) {
        self.multipath_base_avatars
            .insert(proto_character.base_avatar_id, proto_character.clone());

        // Try to resolve any multipath characters that have this as their base avatar.
        for unresolved_avatar_id in self.unresolved_multipath_characters.clone().iter() {
            self.resolve_multipath_character(*unresolved_avatar_id);
        }
    }

    pub fn ingest_multipath_character(&mut self, proto_multipath_character: &AvatarPathData) -> Option<Character> {
        let Some(character) = export_proto_multipath_character(self.database, proto_multipath_character) else {
            warn!(
                uid = &proto_multipath_character.avatar_id,
                "multipath character config not found, skipping"
            );
            return None;
        };

        let character_id = character.id;

        // If it's the trailblazer, determine the gender
        if character.name == "Trailblazer" {
            self.trailblazer = Some(if character_id % 2 == 0 { "Stelle" } else { "Caelus" });
        }

        self.characters.insert(character_id, character);

        let resolved = self.resolve_multipath_character(character_id);
        if resolved.is_none() {
            debug!(uid = &character_id, "multipath character not resolved");
            self.unresolved_multipath_characters.insert(character_id);
        }

        resolved
    }

    pub fn handle_characters(&mut self, characters: GetAvatarDataScRsp) {
        info!(num = characters.avatar_list.len(), "found characters");
        for character in characters.avatar_list {
            self.ingest_character(&character);
        }

        info!(num = characters.avatar_path_data_info_list.len(), "found multipath characters");
        for multipath_avatar_info in characters.avatar_path_data_info_list {
            self.ingest_multipath_character(&multipath_avatar_info);
        }
    }

    fn get_multipath_base_id(&self, avatar_id: u32) -> u32 {
        self.database
            .multipath_avatar_config
            .get(&avatar_id)
            .map(|cfg| cfg.BaseAvatarID)
            .unwrap_or(avatar_id)
    }

    /// Fills in the level and ascension of a character, which are stored in its base avatar
    pub fn resolve_multipath_character(&mut self, character_id: u32) -> Option<Character> {
        let base_avatar_id = self.get_multipath_base_id(character_id);
        let Some(character) = self.characters.get_mut(&character_id) else {
            warn!(uid = &character_id, "multipath character not found");
            return None;
        };

        let base_avatar = self.multipath_base_avatars.get(&base_avatar_id)?;
        character.level = base_avatar.level;
        character.ascension = base_avatar.promotion;

        self.unresolved_multipath_characters.remove(&character_id);

        Some(character.clone())
    }

    pub fn handle_player_sync(&mut self, sync: PlayerSyncScNotify) -> Vec<OptimizerEvent> {
        let mut events = Vec::new();

        let (relics, light_cones, materials) = self.ingest_items(&sync.relic_list, &sync.equipment_list, &sync.material_list);

        if !relics.is_empty() {
            info!(num = relics.len(), "found updated relics");
            events.push(OptimizerEvent::UpdateRelics(relics));
        }

        if !light_cones.is_empty() {
            info!(num = light_cones.len(), "found updated light cones");
            events.push(OptimizerEvent::UpdateLightCones(light_cones));
        }

        if !materials.is_empty() {
            info!(num = materials.len(), "found updated materials");
            events.push(OptimizerEvent::UpdateMaterials(materials));
        }

        if let Some(basic_info) = sync.basic_info.into_option() {
            self.gacha.oneric_shards = basic_info.oneric_shard_count;
            self.gacha.stellar_jade = basic_info.stellar_jade_count;

            events.push(OptimizerEvent::UpdateGachaFunds(self.gacha));
        }

        if !sync.del_relic_list.is_empty() {
            info!(num = sync.del_relic_list.len(), "found deleted relics");
            for del_relic in sync.del_relic_list.iter() {
                if self.relics.remove(del_relic).is_none() {
                    warn!(uid = &del_relic, "del_relic not found");
                }
            }

            events.push(OptimizerEvent::DeleteRelics(sync.del_relic_list));
        }

        if !sync.del_equipment_list.is_empty() {
            info!(num = sync.del_equipment_list.len(), "found deleted light cones");
            for del_light_cone in sync.del_equipment_list.iter() {
                if self.light_cones.remove(del_light_cone).is_none() {
                    warn!(uid = &del_light_cone, "del_light_cone not found");
                }
            }

            events.push(OptimizerEvent::DeleteLightCones(sync.del_equipment_list));
        }

        let mut updated_characters = Vec::new();

        if let Some(avatar_sync) = sync.avatar_sync.into_option() {
            for avatar in avatar_sync.avatar_list {
                self.ingest_character(&avatar)
            }

            for multipath_avatar_info in avatar_sync.avatar_path_data_info_list {
                if let Some(character) = self.ingest_multipath_character(&multipath_avatar_info) {
                    updated_characters.push(character);
                } else {
                    warn!(uid = &multipath_avatar_info.avatar_id, "multipath character not resolved");
                }
            }
        }

        if !updated_characters.is_empty() {
            info!(num = updated_characters.len(), "found updated characters");
            events.push(OptimizerEvent::UpdateCharacters(updated_characters));
        }

        events
    }

    pub fn handle_set_avatar_enhanced(&mut self, set_avatar_enhanced: SetAvatarEnhancedIdScRsp) -> Option<OptimizerEvent> {
        let Some(character) = self.characters.get_mut(&set_avatar_enhanced.growth_avatar_id) else {
            warn!(
                uid = &set_avatar_enhanced.growth_avatar_id,
                "character not found when setting enhanced id, skipping"
            );
            return None;
        };

        character.ability_version = set_avatar_enhanced.skilltree_version;

        Some(OptimizerEvent::UpdateCharacters(vec![character.clone()]))
    }

    fn is_lightcone(&self, item_id: u32) -> bool {
        self.database.equipment_config.get(&item_id).is_some()
    }

    /// The star rating of a light cone or character that can be pulled from a banner
    fn gacha_item_grade(&self, item_id: u32) -> Option<u32> {
        let rarity = if let Some(lc_config) = self.database.equipment_config.get(&item_id) {
            &lc_config.Rarity
        } else {
            &self.database.avatar_config.get(&item_id)?.Rarity
        };

        match rarity.as_str() {
            "CombatPowerLightconeRarity5" | "CombatPowerAvatarRarityType5" => Some(5),
            "CombatPowerLightconeRarity4" | "CombatPowerAvatarRarityType4" => Some(4),
            "CombatPowerLightconeRarity3" => Some(3),
            _ => None,
        }
    }

    pub fn handle_gacha_info(&mut self, gacha_info: GetGachaInfoScRsp) {
        for banner in gacha_info.gacha_info_list {
            let banner_type = match banner.gacha_id {
                1001 => BannerType::Standard,
                _ if banner.prize_item_list.first().is_some_and(|item_id| self.is_lightcone(*item_id)) => BannerType::LightCone,
                _ => BannerType::Character,
            };

            self.banners.insert(
                banner.gacha_id,
                BannerInfo {
                    rate_up_item_list: banner.item_detail_list,
                    banner_type,
                },
            );
        }
    }

    pub fn handle_gacha(&mut self, gacha: DoGachaScRsp) -> Option<OptimizerEvent> {
        let Some(banner) = self.banners.get(&gacha.gacha_id) else {
            warn!(gacha_id = &gacha.gacha_id, "gacha info not found");
            return None;
        };

        let mut gacha_result = GachaResult {
            banner_id: gacha.gacha_id,
            banner_type: banner.banner_type,
            pity_4: PityUpdate::AddPity { amount: 0 },
            pity_5: PityUpdate::AddPity { amount: 0 },
            pull_results: Vec::new(),
        };

        for item in gacha.gacha_item_list {
            let item_id = item.gacha_item.item_id;
            gacha_result.pull_results.push(item_id);

            // Guessing the rarity would report a wrong pity, which is worse than not reporting the pull
            let Some(grade) = self.gacha_item_grade(item_id) else {
                warn!(item_id, "pulled an item of unknown rarity, skipping gacha result");
                return None;
            };

            let was_rate_up = banner.rate_up_item_list.contains(&item_id);
            let next_is_guarantee = !was_rate_up;

            match grade {
                5 => {
                    gacha_result.pity_4.increment();
                    gacha_result.pity_5.reset(next_is_guarantee);
                }
                4 => {
                    gacha_result.pity_4.reset(next_is_guarantee);
                    gacha_result.pity_5.increment();
                }
                _ => {
                    gacha_result.pity_4.increment();
                    gacha_result.pity_5.increment();
                }
            }
        }

        Some(OptimizerEvent::GachaResult(gacha_result))
    }
}
