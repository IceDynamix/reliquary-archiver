use reliquary::network::command::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::command::proto::AvatarPathData::AvatarPathData;
use reliquary::network::command::proto::AvatarPathSkillTree::AvatarPathSkillTree;
use reliquary::network::command::proto::DoGachaScRsp::DoGachaScRsp;
use reliquary::network::command::proto::GachaInfo::GachaInfo;
use reliquary::network::command::proto::GachaItem::GachaItem;
use reliquary::network::command::proto::GetGachaInfoScRsp::GetGachaInfoScRsp;
use reliquary::network::command::proto::Item::Item;
use reliquary::network::command::proto::PlayerSyncScNotify::PlayerSyncScNotify;
use reliquary::network::command::proto::Relic::Relic as ProtoRelic;
use reliquary::network::command::proto::RelicAffix::RelicAffix;
use reliquary::network::command::proto::SetAvatarEnhancedIdScRsp::SetAvatarEnhancedIdScRsp;

use super::*;
use crate::export::Exporter;
use crate::export::database::get_database;

// Long-standing game data that the embedded database is expected to contain
const MARCH_7TH: u32 = 1001; // 4* character
const HIMEKO: u32 = 1003; // 5* character
const ARROWS: u32 = 20000; // 3* light cone
const TWO_STAR_HEAD: u32 = 31011; // relic of set 101
const CHARACTER_BANNER: u32 = 2001;

fn relic(unique_id: u32, main_affix_id: u32) -> ProtoRelic {
    ProtoRelic {
        unique_id,
        tid: TWO_STAR_HEAD,
        main_affix_id,
        sub_affix_list: vec![RelicAffix {
            affix_id: 1,
            cnt: 2,
            step: 1,
            ..Default::default()
        }],
        ..Default::default()
    }
}

fn path_data(avatar_id: u32) -> AvatarPathData {
    AvatarPathData {
        avatar_id,
        rank: 2,
        ..Default::default()
    }
}

fn base_avatar(base_avatar_id: u32, level: u32, promotion: u32) -> ProtoCharacter {
    ProtoCharacter {
        base_avatar_id,
        level,
        promotion,
        ..Default::default()
    }
}

fn pull(item_ids: &[u32]) -> DoGachaScRsp {
    DoGachaScRsp {
        gacha_id: CHARACTER_BANNER,
        gacha_item_list: item_ids
            .iter()
            .map(|&item_id| GachaItem {
                gacha_item: Some(Item {
                    item_id,
                    ..Default::default()
                })
                .into(),
                ..Default::default()
            })
            .collect(),
        ..Default::default()
    }
}

fn exporter_with_banner(rate_up: u32) -> OptimizerExporter {
    let mut exporter = OptimizerExporter::new();
    exporter.handle_gacha_info(GetGachaInfoScRsp {
        gacha_info_list: vec![GachaInfo {
            gacha_id: CHARACTER_BANNER,
            prize_item_list: vec![rate_up],
            item_detail_list: vec![rate_up],
            ..Default::default()
        }],
        ..Default::default()
    });
    exporter
}

#[test]
fn unknown_game_strings_are_not_mapped() {
    assert_eq!(slot_type_to_export("HEAD"), Some("Head"));
    assert_eq!(slot_type_to_export("TAIL"), None);
    assert_eq!(main_stat_to_export("SpeedDelta"), Some("SPD"));
    assert_eq!(main_stat_to_export("LuckDelta"), None);
    assert_eq!(sub_stat_to_export("HPAddedRatio"), Some("HP_"));
    assert_eq!(sub_stat_to_export("LuckDelta"), None);
}

#[test]
fn relic_is_converted() {
    let relic = export_proto_relic(get_database(), &relic(7, 1)).expect("relic should convert");

    assert_eq!(relic._uid, 7);
    assert_eq!(relic.set_id, 101);
    assert_eq!(relic.slot, "Head");
    assert_eq!(relic.rarity, 2);
    assert_eq!(relic.mainstat, "HP");
    assert_eq!(relic.substats.len(), 1);
    assert_eq!(relic.substats[0].key, "HP");
    assert_eq!(relic.substats[0].count, 2);
    assert!(relic.reroll_substats.is_none());
}

#[test]
fn relic_with_unknown_main_affix_is_skipped() {
    assert!(export_proto_relic(get_database(), &relic(7, 9999)).is_none());
}

#[test]
fn skill_tree_points_are_mapped() {
    let point = |point_id, level| AvatarPathSkillTree {
        point_id,
        level,
        ..Default::default()
    };

    let (skills, traces, memosprite) =
        export_skill_tree(&[point(1, 6), point(3, 10), point(6, 1), point(18, 1), point(999, 1), point(0, 5)]);

    assert_eq!((skills.basic, skills.skill, skills.ult, skills.talent), (6, 0, 10, 0));
    assert_eq!(skills.elation, None);
    assert!(traces.ability_1 && traces.stat_10);
    assert!(!traces.ability_2 && !traces.special);
    assert!(memosprite.is_none());

    let (skills, _, memosprite) = export_skill_tree(&[point(19, 3), point(22, 2)]);
    assert_eq!(memosprite.map(|m| (m.skill, m.talent)), Some((3, 0)));
    assert_eq!(skills.elation, Some(2));
}

#[test]
fn character_gets_its_level_once_the_base_avatar_is_known() {
    let mut exporter = OptimizerExporter::new();

    assert!(exporter.ingest_multipath_character(&path_data(MARCH_7TH)).is_none());
    assert_eq!(exporter.characters[&MARCH_7TH].level, 0);

    exporter.ingest_character(&base_avatar(MARCH_7TH, 80, 6));

    let character = &exporter.export().unwrap().characters[0];
    assert_eq!(
        (character.id, character.level, character.ascension, character.eidolon),
        (MARCH_7TH, 80, 6, 2)
    );
}

#[test]
fn character_arriving_after_its_base_avatar_is_resolved_immediately() {
    let mut exporter = OptimizerExporter::new();
    exporter.ingest_character(&base_avatar(MARCH_7TH, 50, 3));

    let character = exporter
        .ingest_multipath_character(&path_data(MARCH_7TH))
        .expect("should be resolved");
    assert_eq!(character.level, 50);
}

#[test]
fn enhanced_ability_version_shows_up_in_export() {
    let mut exporter = OptimizerExporter::new();
    exporter.ingest_character(&base_avatar(MARCH_7TH, 80, 6));
    exporter.ingest_multipath_character(&path_data(MARCH_7TH));

    let event = exporter.handle_set_avatar_enhanced(SetAvatarEnhancedIdScRsp {
        growth_avatar_id: MARCH_7TH,
        skilltree_version: 1,
        ..Default::default()
    });

    assert!(matches!(event, Some(OptimizerEvent::UpdateCharacters(characters)) if characters[0].ability_version == 1));
    assert_eq!(exporter.export().unwrap().characters[0].ability_version, 1);

    let unknown = exporter.handle_set_avatar_enhanced(SetAvatarEnhancedIdScRsp {
        growth_avatar_id: 4242,
        ..Default::default()
    });
    assert!(unknown.is_none());
}

#[test]
fn player_sync_updates_and_deletes_relics() {
    let mut exporter = OptimizerExporter::new();

    let events = exporter.handle_player_sync(PlayerSyncScNotify {
        relic_list: vec![relic(1, 1), relic(2, 1), relic(3, 9999)],
        ..Default::default()
    });
    assert!(matches!(events.as_slice(), [OptimizerEvent::UpdateRelics(relics)] if relics.len() == 2));
    assert_eq!(exporter.relics.len(), 2);

    let events = exporter.handle_player_sync(PlayerSyncScNotify {
        del_relic_list: vec![1],
        ..Default::default()
    });
    assert!(matches!(events.as_slice(), [OptimizerEvent::DeleteRelics(ids)] if ids == &[1]));
    assert_eq!(exporter.relics.keys().copied().collect::<Vec<_>>(), vec![2]);
}

#[test]
fn gacha_pity_follows_the_pulled_rarities() {
    let mut exporter = exporter_with_banner(HIMEKO);

    let Some(OptimizerEvent::GachaResult(result)) = exporter.handle_gacha(pull(&[ARROWS, ARROWS, MARCH_7TH, ARROWS])) else {
        panic!("expected a gacha result");
    };

    assert!(matches!(result.banner_type, BannerType::Character));
    assert_eq!(result.pull_results, vec![ARROWS, ARROWS, MARCH_7TH, ARROWS]);
    assert!(matches!(result.pity_5, PityUpdate::AddPity { amount: 4 }));
    // March 7th is not the rate up, so the next 4* is guaranteed to be one
    assert!(matches!(
        result.pity_4,
        PityUpdate::ResetPity {
            amount: 1,
            set_guarantee: true
        }
    ));

    let Some(OptimizerEvent::GachaResult(result)) = exporter.handle_gacha(pull(&[HIMEKO])) else {
        panic!("expected a gacha result");
    };
    assert!(matches!(
        result.pity_5,
        PityUpdate::ResetPity {
            amount: 0,
            set_guarantee: false
        }
    ));
}

#[test]
fn gacha_result_with_unknown_item_is_dropped() {
    let mut exporter = exporter_with_banner(HIMEKO);
    assert!(exporter.handle_gacha(pull(&[ARROWS, 999_999])).is_none());
}

#[test]
fn gacha_on_unknown_banner_is_dropped() {
    let mut exporter = OptimizerExporter::new();
    assert!(exporter.handle_gacha(pull(&[ARROWS])).is_none());
}

#[test]
fn banner_without_prizes_does_not_panic() {
    let mut exporter = OptimizerExporter::new();
    exporter.handle_gacha_info(GetGachaInfoScRsp {
        gacha_info_list: vec![GachaInfo {
            gacha_id: CHARACTER_BANNER,
            ..Default::default()
        }],
        ..Default::default()
    });
    assert!(matches!(exporter.banners[&CHARACTER_BANNER].banner_type, BannerType::Character));
}
