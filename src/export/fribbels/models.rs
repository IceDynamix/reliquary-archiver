use serde::{Deserialize, Serialize};
use serde_with::{serde_as, DisplayFromStr};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Export {
    pub source: &'static str,
    pub build: &'static str,
    pub version: u32,
    pub metadata: Metadata,
    pub gacha: GachaFunds,
    pub materials: Vec<Material>,
    pub light_cones: Vec<LightCone>,
    pub relics: Vec<Relic>,
    pub characters: Vec<Character>,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Metadata {
    pub uid: Option<u32>,
    pub trailblazer: Option<&'static str>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Copy, Default)]
pub struct GachaFunds {
    pub stellar_jade: u32,
    pub oneric_shards: u32,
}

#[derive(Serialize, Debug, Clone, Copy)]
pub enum BannerType {
    Character,
    LightCone,
    Standard,
}

#[derive(Debug, Clone)]
pub struct BannerInfo {
    pub rate_up_item_list: Vec<u32>,
    pub banner_type: BannerType,
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
#[serde(tag = "event", content = "data")]
pub enum OptimizerEvent {
    InitialScan(Export),
    GachaResult(GachaResult),
    UpdateGachaFunds(GachaFunds),
    UpdateMaterials(Vec<Material>),
    UpdateLightCones(Vec<LightCone>),
    UpdateRelics(Vec<Relic>),
    UpdateCharacters(Vec<Character>),
    DeleteRelics(#[serde_as(as = "Vec<DisplayFromStr>")] Vec<u32>),
    DeleteLightCones(#[serde_as(as = "Vec<DisplayFromStr>")] Vec<u32>),
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
pub struct GachaResult {
    #[serde_as(as = "DisplayFromStr")]
    pub banner_id: u32,
    pub banner_type: BannerType,
    pub pity_4: PityUpdate,
    pub pity_5: PityUpdate,
    #[serde_as(as = "Vec<DisplayFromStr>")]
    pub pull_results: Vec<u32>,
}

#[derive(Serialize, Debug, Clone, Copy)]
#[serde(tag = "kind")]
pub enum PityUpdate {
    AddPity {
        amount: u32,
    },
    ResetPity {
        amount: u32,
        set_guarantee: bool,
    },
}

impl PityUpdate {
    pub fn increment(&mut self) {
        match self {
            PityUpdate::AddPity { amount, .. } => *amount += 1,
            PityUpdate::ResetPity { amount, .. } => *amount += 1,
        }
    }

    pub fn reset(&mut self, guarantee: bool) {
        *self = PityUpdate::ResetPity {
            amount: 0,
            set_guarantee: guarantee,
        }
    }
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Material {
    #[serde_as(as = "DisplayFromStr")]
    pub id: u32,
    pub name: String,
    pub count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<u64>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Relic {
    #[serde_as(as = "DisplayFromStr")]
    pub set_id: u32,
    pub name: String,
    pub slot: &'static str,
    pub rarity: u32,
    pub level: u32,
    pub mainstat: String,
    pub substats: Vec<Substat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reroll_substats: Option<Vec<Substat>>,
    pub location: String,
    pub lock: bool,
    pub discard: bool,
    #[serde_as(as = "DisplayFromStr")]
    pub _uid: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Substat {
    pub key: String,
    pub value: f32,
    pub count: u32,
    pub step: u32,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct LightCone {
    #[serde_as(as = "DisplayFromStr")]
    pub id: u32,
    pub name: String,
    pub level: u32,
    pub ascension: u32,
    pub superimposition: u32,
    pub location: String,
    pub lock: bool,
    #[serde_as(as = "DisplayFromStr")]
    pub _uid: u32,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Character {
    #[serde_as(as = "DisplayFromStr")]
    pub id: u32,
    pub name: String,
    pub path: String,
    pub level: u32,
    pub ascension: u32,
    pub eidolon: u32,
    pub skills: Skills,
    pub traces: Traces,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memosprite: Option<Memosprite>,
    pub ability_version: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Skills {
    pub basic: u32,
    pub skill: u32,
    pub ult: u32,
    pub talent: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct Memosprite {
    pub skill: u32,
    pub talent: u32,
}

impl Memosprite {
    pub fn if_present(self) -> Option<Memosprite> {
        if self.skill == 0 && self.talent == 0 {
            None
        } else {
            Some(self)
        }
    }
} 