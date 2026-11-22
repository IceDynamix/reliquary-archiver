use std::ops::Not;

use serde::{Deserialize, Serialize};
use serde_with::{DisplayFromStr, serde_as};

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
    pub recon_relics: Vec<u32>,
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

#[derive(Serialize, Deserialize, Debug, Clone, Copy)]
pub enum BannerType {
    Character,
    LightCone,
    Standard,
}

#[derive(Serialize, Deserialize, Debug, Clone)]
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
    RelicBox(RelicBoxEvent),
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
    AddPity { amount: u32 },
    ResetPity { amount: u32, set_guarantee: bool },
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
#[derive(Serialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum RelicBoxEvent {
    /// Fired when a user confirms their choice of relic
    Confirmed,
    /// Fired when a user opens a box and is prompted to choose a relic
    Open { relic: BoxRelic },
    /// Fired when a user opens a box but does not need to choose a relic (i.e. first 6 relics of each event)
    Add {
        #[serde_as(as = "DisplayFromStr")]
        relic_id: u32,
    },
}

#[serde_as]
#[derive(Serialize, Debug, Clone)]
pub struct BoxRelic {
    #[serde_as(as = "DisplayFromStr")]
    pub set_id: u32,
    pub name: String,
    pub slot: &'static str,
    pub rarity: u32,
    pub level: u32,
    pub mainstat: String,
    pub substats: Vec<Substat>,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Material {
    #[serde_as(as = "DisplayFromStr")]
    pub id: u32,
    pub name: String,
    pub count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expire_time: Option<u64>,
}

/// A relic slot as the export names it. An alias because serde takes a field written `&str` to
/// borrow from what it reads, which would keep relics from being read back from kept state
pub type SlotName = &'static str;

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Relic {
    #[serde_as(as = "DisplayFromStr")]
    pub set_id: u32,
    pub name: String,
    #[serde(deserialize_with = "super::utils::deserialize_slot")]
    pub slot: SlotName,
    pub rarity: u32,
    pub level: u32,
    pub mainstat: String,
    pub substats: Vec<Substat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reroll_substats: Option<Vec<Substat>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_substats: Option<Vec<Substat>>,
    pub location: String,
    pub lock: bool,
    pub discard: bool,
    #[serde_as(as = "DisplayFromStr")]
    pub _uid: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Substat {
    pub key: String,
    pub value: f32,
    pub count: u32,
    pub step: u32,
}

#[serde_as]
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
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
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
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
    #[serde(skip_serializing_if = "Option::is_none")]
    pub affinitive_eidolons: Option<Vec<u32>>,
    pub ability_version: u32,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Skills {
    pub basic: u32,
    pub skill: u32,
    pub ult: u32,
    pub talent: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub elation: Option<u32>,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
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
    pub special: bool,
    #[serde(default, skip_serializing_if = "<&bool>::not")]
    pub elation_debt: bool,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct Memosprite {
    pub skill: u32,
    pub talent: u32,
}

impl Memosprite {
    pub fn if_present(self) -> Option<Memosprite> {
        if self.skill == 0 && self.talent == 0 { None } else { Some(self) }
    }
}

/// What an exporter has captured, in its own terms rather than the game's: kept between runs, a
/// new game version changes the commands but not this. Change [`ExporterState::VERSION`] along
/// with anything here that older states wouldn't read right.
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ExporterState {
    pub version: u32,
    pub initialized: bool,
    pub uid: Option<u32>,
    pub trailblazer: Option<String>,
    pub banners: std::collections::HashMap<u32, BannerInfo>,
    pub gacha: GachaFunds,
    pub materials: Vec<Material>,
    pub light_cones: Vec<LightCone>,
    pub relics: Vec<Relic>,
    pub recon_relics: Vec<u32>,
    pub characters: Vec<Character>,
    pub base_avatars: std::collections::HashMap<u32, BaseAvatar>,
    pub unresolved_multipath_characters: Vec<u32>,
}

impl ExporterState {
    pub const VERSION: u32 = 1;
}

/// What multipath characters take from their base avatar
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct BaseAvatar {
    pub level: u32,
    pub ascension: u32,
}
