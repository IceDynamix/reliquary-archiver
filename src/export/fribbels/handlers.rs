use crate::export::fribbels::converters::*;
use crate::export::fribbels::models::*;

use protobuf::Enum;
use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::proto::DoGachaScRsp::DoGachaScRsp;
use reliquary::network::command::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::command::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::command::proto::GetGachaInfoScRsp::GetGachaInfoScRsp;
use reliquary::network::command::proto::MultiPathAvatarInfo::MultiPathAvatarInfo;
use reliquary::network::command::proto::MultiPathAvatarType::MultiPathAvatarType;
use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use reliquary::network::command::proto::PlayerLoginScRsp::PlayerLoginScRsp;
use reliquary::network::command::proto::PlayerSyncScNotify::PlayerSyncScNotify;
use reliquary::network::command::proto::SetAvatarEnhancedIdScRsp::SetAvatarEnhancedIdScRsp;
use tracing::{debug, info, warn};

use super::OptimizerExporter;

impl OptimizerExporter {
    pub fn handle_token(&mut self, token: PlayerGetTokenScRsp) {
        self.uid = Some(token.uid);
    }

    pub fn handle_login(&mut self, login: PlayerLoginScRsp) {
        self.gacha.oneric_shards = login.basic_info.oneric_shard_count;
        self.gacha.stellar_jade = login.basic_info.stellar_jade_count;
    }

    pub fn handle_inventory(&mut self, bag: GetBagScRsp) {
        let relics: Vec<Relic> = bag
            .relic_list
            .iter()
            .filter_map(|r| export_proto_relic(&self.database, r))
            .collect();

        info!(num = relics.len(), "found relics");
        for relic in &relics {
            self.relics.insert(relic._uid, relic.clone());
        }

        let light_cones: Vec<LightCone> = bag
            .equipment_list
            .iter()
            .filter_map(|equip| export_proto_light_cone(&self.database, equip))
            .collect();

        info!(num = light_cones.len(), "found light cones");
        for light_cone in light_cones {
            self.light_cones.insert(light_cone._uid, light_cone);
        }

        let materials: Vec<Material> = bag
            .material_list
            .iter()
            .filter_map(|m| export_proto_material(&self.database, m))
            .collect();

        info!(num = materials.len(), "found materials");
        for material in materials {
            self.materials.insert(material.id, material);
        }
    }

    pub fn ingest_character(&mut self, proto_character: &ProtoCharacter) -> Option<Character> {
        let Some(character) = export_proto_character(&self.database, proto_character) else {
            warn!(
                uid = &proto_character.base_avatar_id,
                "character config not found, skipping"
            );
            return None;
        };

        if MultiPathAvatarType::from_i32(proto_character.base_avatar_id as i32).is_some() {
            self.multipath_base_avatars
                .insert(proto_character.base_avatar_id, proto_character.clone());

            // Try to resolve any multipath characters that have this as their base avatar.
            for unresolved_avatar_id in self.unresolved_multipath_characters.clone().iter() {
                self.resolve_multipath_character(*unresolved_avatar_id);
            }

            return None;
        } else {
            // Only emit character data here if it's not a multipath character.
            // For multipath characters, we need to wait for the multipath packet
            // to get the rest of the data, so we'll do that in [ingest_multipath_character].

            self.characters.insert(character.id, character.clone());

            return Some(character);
        }
    }

    pub fn ingest_multipath_character(
        &mut self,
        proto_multipath_character: &MultiPathAvatarInfo,
    ) -> Option<Character> {
        let Some(character) =
            export_proto_multipath_character(&self.database, proto_multipath_character)
        else {
            warn!(
                uid = &proto_multipath_character.avatar_id.value(),
                "multipath character config not found, skipping"
            );
            return None;
        };

        self.multipath_characters
            .insert(character.id, character.clone());

        // If it's the trailblazer, determine the gender
        if character.name == "Trailblazer" {
            self.trailblazer = Some(if character.id % 2 == 0 {
                "Stelle"
            } else {
                "Caelus"
            });
        }

        if let Some(character) = self.resolve_multipath_character(character.id) {
            return Some(character);
        } else {
            debug!(uid = &character.id, "multipath character not resolved");
            self.unresolved_multipath_characters.insert(character.id);

            return None;
        }
    }

    pub fn handle_characters(&mut self, characters: GetAvatarDataScRsp) {
        info!(num = characters.avatar_list.len(), "found characters");
        for character in characters.avatar_list {
            self.ingest_character(&character);
        }

        info!(
            num = characters.multi_path_avatar_info_list.len(),
            "found multipath characters"
        );
        for multipath_avatar_info in characters.multi_path_avatar_info_list {
            self.ingest_multipath_character(&multipath_avatar_info);
        }
    }

    fn get_multipath_base_id(&self, avatar_id: u32) -> u32 {
        self.database
            .multipath_avatar_config
            .get(&avatar_id)
            .expect("multipath character not found")
            .BaseAvatarID
    }

    pub fn resolve_multipath_character(&mut self, character_id: u32) -> Option<Character> {
        let base_avatar_id = self.get_multipath_base_id(character_id);
        let Some(character) = self.multipath_characters.get_mut(&character_id) else {
            warn!(uid = &character_id, "multipath character not found");
            return None;
        };

        if let Some(base_avatar) = self.multipath_base_avatars.get(&base_avatar_id) {
            character.level = base_avatar.level;
            character.ascension = base_avatar.promotion;

            self.unresolved_multipath_characters.remove(&character_id);

            return Some(character.clone());
        }

        return None;
    }

    pub fn handle_player_sync(&mut self, sync: PlayerSyncScNotify) -> Vec<OptimizerEvent> {
        let mut events = Vec::new();

        let relics: Vec<Relic> = sync
            .relic_list
            .iter()
            .filter_map(|r| export_proto_relic(&self.database, r))
            .collect();

        if !relics.is_empty() {
            info!(num = relics.len(), "found updated relics");
            for relic in relics.clone() {
                self.relics.insert(relic._uid, relic);
            }

            events.push(OptimizerEvent::UpdateRelics(relics));
        }

        let light_cones: Vec<LightCone> = sync
            .equipment_list
            .iter()
            .filter_map(|equip| export_proto_light_cone(&self.database, equip))
            .collect();

        if !light_cones.is_empty() {
            info!(num = light_cones.len(), "found updated light cones");
            for light_cone in light_cones.clone() {
                self.light_cones.insert(light_cone._uid, light_cone);
            }

            events.push(OptimizerEvent::UpdateLightCones(light_cones));
        }

        let materials: Vec<Material> = sync
            .material_list
            .iter()
            .filter_map(|m| export_proto_material(&self.database, m))
            .collect();

        if !materials.is_empty() {
            info!(num = materials.len(), "found updated materials");
            for material in materials.clone() {
                self.materials.insert(material.id, material);
            }

            events.push(OptimizerEvent::UpdateMaterials(materials));
        }

        if let Some(basic_info) = sync.basic_info.into_option() {
            self.gacha.oneric_shards = basic_info.oneric_shard_count;
            self.gacha.stellar_jade = basic_info.stellar_jade_count;

            events.push(OptimizerEvent::UpdateGachaFunds(self.gacha.clone()));
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
            info!(
                num = sync.del_equipment_list.len(),
                "found deleted light cones"
            );
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
                if let Some(character) = self.ingest_character(&avatar) {
                    updated_characters.push(character);
                }
            }
        }

        if !sync.multi_path_avatar_info_list.is_empty() {
            for multipath_avatar_info in sync.multi_path_avatar_info_list {
                if let Some(character) = self.ingest_multipath_character(&multipath_avatar_info) {
                    updated_characters.push(character);
                } else {
                    warn!(
                        uid = &multipath_avatar_info.avatar_id.value(),
                        "multipath character not resolved"
                    );
                }
            }
        }

        if !updated_characters.is_empty() {
            info!(num = updated_characters.len(), "found updated characters");
            events.push(OptimizerEvent::UpdateCharacters(updated_characters));
        }

        events
    }

    pub fn handle_set_avatar_enhanced(
        &mut self,
        set_avatar_enhanced: SetAvatarEnhancedIdScRsp,
    ) -> OptimizerEvent {
        let Some(character) = self
            .characters
            .get_mut(&set_avatar_enhanced.growth_avatar_id)
        else {
            warn!(
                uid = &set_avatar_enhanced.growth_avatar_id,
                "character not found when setting enhanced id, skipping"
            );
            return OptimizerEvent::UpdateCharacters(vec![]);
        };

        character.ability_version = set_avatar_enhanced.skilltree_version;

        OptimizerEvent::UpdateCharacters(vec![character.clone()])
    }

    fn is_lightcone(&self, item_id: u32) -> bool {
        self.database.equipment_config.get(&item_id).is_some()
    }

    pub fn handle_gacha_info(&mut self, gacha_info: GetGachaInfoScRsp) {
        for banner in gacha_info.gacha_info_list {
            self.banners.insert(
                banner.gacha_id,
                BannerInfo {
                    rate_up_item_list: banner.item_detail_list,
                    banner_type: match banner.gacha_id {
                        1001 => BannerType::Standard,
                        _ => {
                            if self.is_lightcone(*banner.prize_item_list.first().unwrap()) {
                                BannerType::LightCone
                            } else {
                                BannerType::Character
                            }
                        }
                    },
                },
            );
        }
    }

    pub fn handle_gacha(&mut self, gacha: DoGachaScRsp) -> Option<OptimizerEvent> {
        if let Some(banner) = self.banners.get(&gacha.gacha_id) {
            let mut gacha_result = GachaResult {
                banner_id: gacha.gacha_id,
                banner_type: banner.banner_type,
                pity_4: PityUpdate::AddPity { amount: 0 },
                pity_5: PityUpdate::AddPity { amount: 0 },
                pull_results: Vec::new(),
            };

            for item in gacha.gacha_item_list {
                gacha_result.pull_results.push(item.gacha_item.item_id);

                let grade = if let Some(lc_config) =
                    self.database.equipment_config.get(&item.gacha_item.item_id)
                {
                    match lc_config.Rarity.as_str() {
                        "CombatPowerLightconeRarity5" => 5,
                        "CombatPowerLightconeRarity4" => 4,
                        "CombatPowerLightconeRarity3" => 3,
                        _ => panic!("Unknown light cone rarity: {}", lc_config.Rarity),
                    }
                } else if let Some(avatar_config) =
                    self.database.avatar_config.get(&item.gacha_item.item_id)
                {
                    match avatar_config.Rarity.as_str() {
                        "CombatPowerAvatarRarityType5" => 5,
                        "CombatPowerAvatarRarityType4" => 4,
                        _ => panic!("Unknown avatar rarity: {}", avatar_config.Rarity),
                    }
                } else {
                    panic!("item not found: {}", item.gacha_item.item_id);
                };

                let was_rate_up = banner.rate_up_item_list.contains(&item.gacha_item.item_id);
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

            return Some(OptimizerEvent::GachaResult(gacha_result));
        } else {
            warn!(gacha_id = &gacha.gacha_id, "gacha info not found");
            return None;
        }
    }
}
