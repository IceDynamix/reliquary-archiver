use reliquary::network::gen::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::gen::proto::AvatarSkillTree::AvatarSkillTree as ProtoSkillTree;
use serde::{Deserialize, Serialize};
use tracing::{info, info_span};

use crate::export::optimizer::ConfigMaps;

pub fn export_proto_character(config: &ConfigMaps, proto: &ProtoCharacter) -> Character {
    let key = config.avatar_config
        .get(&proto.base_avatar_id).unwrap()
        .AvatarName.lookup(&config.text_map)
        .unwrap()
        .to_string();

    let span = info_span!("character", key, id = proto.base_avatar_id);
    let _enter = span.enter();

    if proto.base_avatar_id >= 8000 {
        info!(?proto);
    }

    info!("skill_ids {}", proto.skilltree_list.iter().map(|s|format!("{},",s.point_id)).collect::<String>());

    let (skills, traces) = export_skill_tree(config, &proto.skilltree_list);


    Character {
        key,
        level: proto.level,
        ascension: proto.promotion,
        eidolon: proto.rank,
        skills,
        traces,
    }
}

fn export_skill_tree(config: &ConfigMaps, proto: &[ProtoSkillTree]) -> (Skills, Traces) {
    let mut skills = Skills {
        basic: 0,
        skill: 0,
        ult: 0,
        talent: 0,
    };

    let mut traces = Traces {
        ability_1: false,
        ability_2: false,
        ability_3: false,
        stat_1: false,
        stat_2: false,
        stat_3: false,
        stat_4: false,
        stat_5: false,
        stat_6: false,
        stat_7: false,
        stat_8: false,
        stat_9: false,
        stat_10: false,
    };

    // skill traces
    for skill in proto.iter() {
        let skill_tree_config = config.avatar_skill_tree_config
            .get(&skill.point_id, &skill.level).unwrap();

        match skill_tree_config.Anchor.as_str() {
            "Point01" => { skills.basic = skill.level }
            "Point02" => { skills.skill = skill.level }
            "Point03" => { skills.ult = skill.level }
            "Point04" => { skills.talent = skill.level }

            "Point05" => { /* technique */ }

            "Point06" => { traces.ability_1 = true }
            "Point07" => { traces.ability_2 = true }
            "Point08" => { traces.ability_3 = true }

            "Point09" => { traces.stat_1 = true }
            "Point10" => { traces.stat_2 = true }
            "Point11" => { traces.stat_3 = true }
            "Point12" => { traces.stat_4 = true }
            "Point13" => { traces.stat_5 = true }
            "Point14" => { traces.stat_6 = true }
            "Point15" => { traces.stat_7 = true }
            "Point16" => { traces.stat_8 = true }
            "Point17" => { traces.stat_9 = true }
            "Point18" => { traces.stat_10 = true }

            _ => { panic!("Unknown point anchor: {}", skill_tree_config.Anchor) }
        }
    }

    (skills, traces)
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Character {
    pub key: String,
    pub level: u32,
    pub ascension: u32,
    pub eidolon: u32,
    pub skills: Skills,
    pub traces: Traces,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Skills {
    pub basic: u32,
    pub skill: u32,
    pub ult: u32,
    pub talent: u32,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Traces {
    pub ability_1: bool,
    pub ability_2: bool,
    pub ability_3: bool,
    pub stat_1: bool,
    pub stat_2: bool,
    pub stat_3: bool,
    pub stat_4: bool,
    pub stat_5: bool,
    pub stat_6: bool,
    pub stat_7: bool,
    pub stat_8: bool,
    pub stat_9: bool,
    pub stat_10: bool,
}