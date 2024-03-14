use reliquary::network::gen::proto::Equipment::Equipment as ProtoLightCone;
use serde::{Deserialize, Serialize};

use crate::export::optimizer::ConfigMaps;

#[derive(Serialize, Deserialize, Debug)]
pub struct LightCone {
    pub key: String,
    pub level: u32,
    pub ascension: u32,
    pub superimposition: u32,
    pub location: String,
    pub lock: bool,
    pub _id: String,
}

pub fn export_proto_light_cone(config: &ConfigMaps, proto: &ProtoLightCone) -> LightCone {
    let key = config.equipment_config
        .get(&proto.tid).unwrap()
        .EquipmentName.lookup(&config.text_map)
        .unwrap()
        .to_string();

    let location = if proto.base_avatar_id != 0 {
        config.avatar_config
            .get(&proto.base_avatar_id).unwrap()
            .AvatarName
            .lookup(&config.text_map).unwrap()
            .to_string()
    } else {
        "".to_string()
    };

    LightCone {
        key,
        level: proto.level,
        ascension: proto.promotion,
        superimposition: proto.rank,
        location,
        lock: proto.is_protected,
        _id: proto.unique_id.to_string(),
    }
}