//! Output format based on the format used by [Fribbels HSR Optimizer],
//! devised by [kel-z's HSR-Scanner].
//!
//! [Fribbels HSR Optimizer]: https://github.com/fribbels/hsr-optimizer
//! [kel-z's HSR-Scanner]: https://github.com/kel-z/HSR-Scanner
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::PathBuf;

use base64::Engine;
use base64::prelude::BASE64_STANDARD;
use reliquary::network::GameCommand;
use reliquary::network::gen::command_id;
use reliquary::network::gen::proto::Avatar::Avatar as ProtoCharacter;
use reliquary::network::gen::proto::AvatarSkillTree::AvatarSkillTree as ProtoSkillTree;
use reliquary::network::gen::proto::Equipment::Equipment as ProtoLightCone;
use reliquary::network::gen::proto::Gender::Gender;
use reliquary::network::gen::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::gen::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::gen::proto::GetHeroBasicTypeInfoScRsp::GetHeroBasicTypeInfoScRsp;
use reliquary::network::gen::proto::HeroBasicTypeInfo::HeroBasicTypeInfo;
use reliquary::network::gen::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use reliquary::network::gen::proto::Relic::Relic as ProtoRelic;
use reliquary::network::gen::proto::RelicAffix::RelicAffix;
use reliquary::resource::excel::*;
use reliquary::resource::ResourceMap;
use reliquary::resource::text_map::TextMap;
use serde::{Deserialize, Serialize};
use serde::de::DeserializeOwned;
use serde_json::Value;
use tracing::{debug, info, info_span, instrument, trace, warn};

use crate::export::Exporter;

const BASE_RESOURCE_URL: &str = "https://raw.githubusercontent.com/Dimbreath/StarRailData/master";

#[derive(Serialize, Deserialize, Debug)]
pub struct Export {
    pub source: &'static str,
    pub build: &'static str,
    pub version: u32,
    pub metadata: Metadata,
    pub light_cones: Vec<LightCone>,
    pub relics: Vec<Relic>,
    pub characters: Vec<Character>,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Metadata {
    pub uid: Option<u32>,
    pub trailblazer: Option<&'static str>,
    pub current_trailblazer_path: Option<&'static str>,
}

pub struct OptimizerExporter {
    database: Database,
    uid: Option<u32>,
    trailblazer: Option<&'static str>,
    current_trailblazer_path: Option<&'static str>,
    light_cones: Vec<LightCone>,
    relics: Vec<Relic>,
    characters: Vec<Character>,
    trailblazer_characters: Vec<Character>,
}

impl OptimizerExporter {
    pub fn new(database: Database) -> OptimizerExporter {
        OptimizerExporter {
            database,
            uid: None,
            trailblazer: None,
            current_trailblazer_path: None,
            light_cones: vec![],
            relics: vec![],
            characters: vec![],
            trailblazer_characters: vec![],
        }
    }

    pub fn set_uid(&mut self, uid: u32) {
        self.uid = Some(uid);
    }

    pub fn add_trailblazer_data(&mut self, hero: GetHeroBasicTypeInfoScRsp) {
        let gender = match hero.gender.enum_value().unwrap() {
            Gender::GenderNone => "", // probably in the prologue before selecting gender?
            Gender::GenderMan => "Caelus",
            Gender::GenderWoman => "Stelle"
        };

        self.trailblazer = Some(gender);
        info!(gender, "found trailblazer gender");

        let mut builds: Vec<Character> = hero.basic_type_info_list.iter()
            .filter_map(|b| export_proto_hero(&self.database, &b))
            .collect();

        self.current_trailblazer_path = avatar_path_lookup(&self.database, hero.cur_basic_type.value() as u32);
        if let Some(path) = self.current_trailblazer_path {
            info!(path, "found current trailblazer path");
        } else {
            warn!("unknown path for current trailblazer");
        }

        info!(num=builds.len(), "found trailblazer builds");
        self.trailblazer_characters.append(&mut builds);
    }

    pub fn add_inventory(&mut self, bag: GetBagScRsp) {
        let mut relics: Vec<Relic> = bag.relic_list.iter()
            .filter_map(|r| export_proto_relic(&self.database, r))
            .collect();

        info!(num=relics.len(), "found relics");
        self.relics.append(&mut relics);

        let mut light_cones: Vec<LightCone> = bag.equipment_list.iter()
            .filter_map(|equip| export_proto_light_cone(&self.database, equip))
            .collect();

        info!(num=light_cones.len(), "found light cones");
        self.light_cones.append(&mut light_cones);
    }

    pub fn add_characters(&mut self, characters: GetAvatarDataScRsp) {
        let mut characters: Vec<Character> = characters.avatar_list.iter()
            .filter(|a| a.base_avatar_id < 8000) // skip trailblazer, handled in `write_hero`
            .filter_map(|char| export_proto_character(&self.database, char))
            .collect();

        info!(num=characters.len(), "found characters");
        self.characters.append(&mut characters);
    }
}

impl Exporter for OptimizerExporter {
    type Export = Export;

    fn read_command(&mut self, command: GameCommand) {
        match command.command_id {
            command_id::PlayerGetTokenScRsp => {
                debug!("detected uid");
                let cmd = command.parse_proto::<PlayerGetTokenScRsp>();
                match cmd {
                    Ok(cmd) => {
                        self.set_uid(cmd.uid)
                    }
                    Err(error) => {
                        warn!(%error, "could not parse token command");
                    }
                }
            }
            command_id::GetBagScRsp => {
                debug!("detected inventory packet");
                let cmd = command.parse_proto::<GetBagScRsp>();
                match cmd {
                    Ok(cmd) => {
                        self.add_inventory(cmd)
                    }
                    Err(error) => {
                        warn!(%error, "could not parse inventory data command");
                    }
                }
            }
            command_id::GetAvatarDataScRsp => {
                debug!("detected character packet");
                let cmd = command.parse_proto::<GetAvatarDataScRsp>();
                match cmd {
                    Ok(cmd) => {
                        self.add_characters(cmd)
                    }
                    Err(error) => {
                        warn!(%error, "could not parse character data command");
                    }
                }
            }
            command_id::GetHeroBasicTypeInfoScRsp => {
                debug!("detected trailblazer packet");
                let cmd = command.parse_proto::<GetHeroBasicTypeInfoScRsp>();
                match cmd {
                    Ok(cmd) => {
                        self.add_trailblazer_data(cmd);
                    }
                    Err(error) => {
                        warn!(%error, "could not parse trailblazer data command");
                    }
                }
            }
            _ => {
                trace!(command_id=command.command_id, tag=command.get_command_name(), "ignored");
            }
        }
    }

    fn is_finished(&self) -> bool {
        self.trailblazer.is_some()
            && self.uid.is_some()
            && !self.relics.is_empty()
            && !self.characters.is_empty()
            && !self.trailblazer_characters.is_empty()
            && !self.light_cones.is_empty()
    }

    #[instrument(skip_all)]
    fn export(self) -> Self::Export {
        info!("exporting collected data");

        if self.trailblazer.is_none() {
            warn!("trailblazer gender was not recorded");
        }

        if self.uid.is_none() {
            warn!("uid was not recorded");
        }

        if self.relics.is_empty() {
            warn!("relics were not recorded");
        }

        if self.light_cones.is_empty() {
            warn!("light cones were not recorded");
        }

        if self.trailblazer_characters.is_empty() {
            warn!("trailblazer characters were not recorded");
        }

        if self.characters.is_empty() {
            warn!("characters were not recorded");
        }

        Export {
            source: "reliquary_archiver",
            build: env!("CARGO_PKG_VERSION"),
            version: 3,
            metadata: Metadata {
                uid: self.uid,
                trailblazer: self.trailblazer,
                current_trailblazer_path: self.current_trailblazer_path,
            },
            light_cones: self.light_cones,
            relics: self.relics,
            characters: self.characters.into_iter()
                .chain(self.trailblazer_characters)
                .collect(),
        }
    }
}

#[derive(Serialize, Deserialize)]
struct DatabaseVersion {
    config_sha: String,
    text_map_sha: String,
    keys_sha: String,
}

/**
 * Options struct that get passed to Database::new_from_online()
 * Each `use_` property determines if that item should be fetched from an online source or from a local source
 */
#[derive(Debug, Default)]
struct DatabaseBuildOptions {
    save: bool,
    root_path: PathBuf,
    use_online_config: bool,
    use_online_text_map: bool,
    use_online_keys: bool,
}

impl DatabaseBuildOptions {
    pub fn needs_update(&self) -> bool {
        self.use_online_config || self.use_online_text_map || self.use_online_keys
    }
}

pub struct Database {
    avatar_config: AvatarConfigMap,
    avatar_skill_tree_config: AvatarSkillTreeConfigMap,
    equipment_config: EquipmentConfigMap,
    relic_config: RelicConfigMap,
    relic_set_config: RelicSetConfigMap,
    relic_main_affix_config: RelicMainAffixConfigMap,
    relic_sub_affix_config: RelicSubAffixConfigMap,
    text_map: TextMap,
    keys: HashMap<u32, Vec<u8>>,
}

impl Database {
    fn new_from_online(options: DatabaseBuildOptions) -> Self {
        if options.needs_update() {
            info!("downloading updates, this might take a while...");
        } else {
            info!("loading database...");
        }

        Database {
            avatar_config: Self::load_config(&options),
            avatar_skill_tree_config: Self::load_config(&options),
            equipment_config: Self::load_config(&options),
            relic_config: Self::load_config(&options),
            relic_set_config: Self::load_config(&options),
            relic_main_affix_config: Self::load_config(&options),
            relic_sub_affix_config: Self::load_config(&options),
            text_map: Self::load_text_map(&options),
            keys: Self::load_keys(&options),
        }
    }

    #[instrument(skip_all, name = "config_map")]
    pub fn new_from_source(should_save: bool, save_path: &PathBuf) -> Self {
        info!("checking database version...");

        let database_root = if save_path.to_str().unwrap_or("__temp_dir__") == "__temp_dir__" {
            std::env::temp_dir().join("reliquary-archiver")
        } else {
            save_path.to_owned()
        };

        let fallback_options = DatabaseBuildOptions {
            save: should_save,
            root_path: database_root.clone(),
            use_online_config: true,
            use_online_text_map: true,
            use_online_keys: true,
        };

        // create dir if not exists (and ExcelOutput subdir)
        let excel_path = database_root.join("ExcelOutput");

        if let Err(err) = fs::read_dir(&excel_path) {
            match err.kind() {
                io::ErrorKind::NotFound => {
                    if should_save {
                        if let Err(err) = fs::create_dir_all(excel_path) {
                            // could not create offline directories, fallback to online sources
                            warn!(%err, "unable to create database directory; using online sources");
                            return Self::new_from_online(fallback_options);
                        }
                    } else {
                        // directory does not exist locally and no intent to save locally, so fetch from online sources
                        info!("no local database found; fetching from online sources");
                        return Self::new_from_online(fallback_options);
                    }
                }
                _ => {
                    // could not read offline directory, fallback to online sources
                    warn!(%err, "unable to read database directory; using online sources");
                    return Self::new_from_online(fallback_options);
                }
            }
        }

        // read database version file
        // if it doesn't exist, create it
        let mut file = match File::options().read(true).open(database_root.join("version.json")) {
            Ok(f) => {
                info!("found local database at {}", database_root.to_str().unwrap());
                f
            }
            // could not open file for reading
            Err(err) => {
                match err.kind() {
                    // because it did not exist
                    io::ErrorKind::NotFound => {
                        if should_save {
                            // then create it
                            match File::options().read(true).write(true).create(true).open(database_root.join("version.json")) {
                                Ok(f) => {
                                    info!("creating local database...");
                                    f
                                }
                                Err(err) => {
                                    warn!(%err, "unable to create local database version file; using online sources");
                                    return Self::new_from_online(fallback_options);
                                }
                            }
                        } else {
                            // fetch from online
                            info!("unable to find local database version file; using online sources");
                            return Self::new_from_online(fallback_options);
                        }
                    }
                    // because of other I/O errors
                    _ => {
                        warn!(%err, "unable to open database version file for load; using online sources");
                        return Self::new_from_online(fallback_options); // file system error, fetch from online sources
                    }
                }
            }
        };

        let mut buf: Vec<u8> = Vec::new();

        // read file contents into buffer
        if let Err(err) = file.read_to_end(&mut buf) {
            warn!(%err, "unable to read database version contents; using online sources");
            return Self::new_from_online(fallback_options); // could not read file, fetch from online sources
        }

        // deserialize file contents to DatabaseVersion struct
        let mut version: DatabaseVersion = serde_json::from_slice(&buf)
            .unwrap_or(DatabaseVersion {
                config_sha: String::new(),
                text_map_sha: String::new(),
                keys_sha: String::new(),
            });

        // get the commit history for each of the resources we are interested in
        // todo: probably want to put these strings in a config file/object somewhere instead of hard-coded
        let api_config = "https://api.github.com/repos/Dimbreath/StarRailData/commits?sha=master&path=ExcelOutput".to_string();
        let api_text_map = "https://api.github.com/repos/Dimbreath/StarRailData/commits?sha=master&path=TextMap/TextMapEN.json".to_string();
        let api_keys = "https://api.github.com/repos/tamilpp25/Iridium-SR/commits?sha=main&path=data/Keys.json".to_string();

        let mut options = DatabaseBuildOptions {
            save: should_save,
            root_path: database_root.to_owned(),
            ..Default::default() // initialize all `use_` flags to `false`
        };

        // compare latest commits with local SHA values
        let api_config_res = match Self::get_version(api_config) {
            Some(v) => v,
            None => {
                // could not get API information, fetch this resource from online
                Value::Null
            }
        };

        // first item from response is always the latest commit
        let latest_commit = api_config_res[0]["sha"].as_str().unwrap_or("no data").to_string();
        
        // SHAs don't match, download and update local config files
        if version.config_sha != latest_commit {
            debug!("excel configs out of date");
            
            // only overwrite if we actually got data back
            if latest_commit != "no data" {
                version.config_sha = latest_commit;
            }
            
            options.use_online_config = true;
        }

        let api_text_map_res = match Self::get_version(api_text_map) {
            Some(v) => v,
            None => {
                Value::Null
            }
        };

        let latest_commit = api_text_map_res[0]["sha"].as_str().unwrap_or("no data").to_string();

        // SHAs don't match, download and update local text map file
        if version.text_map_sha != latest_commit {
            debug!("text map out of date");

            if latest_commit != "no data" {
                version.text_map_sha = latest_commit;
            }
            
            options.use_online_text_map = true;
        }

        let api_keys_res = match Self::get_version(api_keys) {
            Some(v) => v,
            None => {
                Value::Null
            }
        };

        let latest_commit = api_keys_res[0]["sha"].as_str().unwrap_or("no data").to_string();

        // SHAs don't match, download and update local keys file
        if version.keys_sha != latest_commit {
            debug!("keys out of date");

            if latest_commit != "no data" {
                version.keys_sha = latest_commit;
            }
            
            options.use_online_keys = true;
        }

        // all SHAs match latest commits, return local database
        if !options.needs_update() {
            info!("local database is current with online sources");
            return Self::new_from_online(options);
        }

        // create database using one or more online sources
        info!("local database requires updates from online sources");
        let database = Self::new_from_online(options);

        // we should only update our version file AFTER we've successfully downloaded the new database files AND the
        // user has indicated they want to save locally
        if should_save {
            // re-open the version file for writing, truncating in the process
            let file = match File::options().write(true).truncate(true).open(database_root.join("version.json")) {
                Ok(f) => {
                    info!("updated local database at {}", database_root.to_str().unwrap());
                    f
                }
                Err(err) => {
                    warn!(%err, "unable to access database version file for save");
                    return database; // don't do any special error-handling here, worst case is we have to re-download on next run
                }
            };

            // update version file and return created database
            match serde_json::to_writer(file, &version) {
                Ok(()) => database,
                Err(err) => {
                    warn!(%err, "unable to update database version file");
                    database
                }
            }
        } else {
            database
        }
    }

    fn load_config<T: ResourceMap + DeserializeOwned + Serialize>(options: &DatabaseBuildOptions) -> T {
        if options.use_online_config {
            Self::load_online_config(&options)
        } else {
            Self::load_offline_config(&options)
        }
    }

    fn load_online_config<T: ResourceMap + DeserializeOwned + Serialize>(options: &DatabaseBuildOptions) -> T {
        let content = Self::get::<T>(format!("{BASE_RESOURCE_URL}/ExcelOutput/{}", T::get_json_name()));

        if options.save {
            let file = File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(options.root_path.join("ExcelOutput").join(T::get_json_name()))
                .unwrap();

            serde_json::to_writer(file, &content).unwrap();
        }

        content
    }

    fn load_offline_config<T: ResourceMap + DeserializeOwned>(options: &DatabaseBuildOptions) -> T {
        Self::get_file::<T>(options.root_path.join("ExcelOutput").join(T::get_json_name()))
    }

    fn load_text_map(options: &DatabaseBuildOptions) -> TextMap {
        if options.use_online_text_map {
            Self::load_online_text_map(&options)
        } else {
            Self::load_offline_text_map(&options)
        }
    }

    fn load_online_text_map(options: &DatabaseBuildOptions) -> TextMap {
        let content = Self::get(format!("{BASE_RESOURCE_URL}/TextMap/TextMapEN.json"));

        if options.save {
            let file = File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(options.root_path.join("TextMapEN.json"))
                .unwrap();

            serde_json::to_writer(file, &content).unwrap();
        }

        content
    }

    fn load_offline_text_map(options: &DatabaseBuildOptions) -> TextMap {
        Self::get_file(options.root_path.join("TextMapEN.json"))
    }

    fn load_keys(options: &DatabaseBuildOptions) -> HashMap<u32, Vec<u8>> {
        if options.use_online_keys {
            Self::load_online_keys(&options)
        } else {
            Self::load_offline_keys(&options)
        }
    }

    fn load_online_keys(options: &DatabaseBuildOptions) -> HashMap<u32, Vec<u8>> {
        let keys: HashMap<u32, String> = Self::get("https://raw.githubusercontent.com/tamilpp25/Iridium-SR/main/data/Keys.json".to_string());

        if options.save {
            let file = File::options()
                .write(true)
                .create(true)
                .truncate(true)
                .open(options.root_path.join("Keys.json"))
                .unwrap();

            serde_json::to_writer(file, &keys).unwrap();
        }

        Self::decode_keys(keys)
    }

    fn load_offline_keys(options: &DatabaseBuildOptions) -> HashMap<u32, Vec<u8>> {
        let mut file = File::options().read(true).open(options.root_path.join("Keys.json")).unwrap();
        let mut content: Vec<u8> = Vec::new();
        file.read_to_end(&mut content).unwrap();

        let keys: HashMap<u32, String> = serde_json::from_slice(&content).unwrap();
        Self::decode_keys(keys)
    }

    fn decode_keys(keys: HashMap<u32, String>) -> HashMap<u32, Vec<u8>> {
        let mut keys_bytes = HashMap::new();

        for (k, v) in keys {
            keys_bytes.insert(k, BASE64_STANDARD.decode(v).unwrap());
        }
        
        keys_bytes
    }

    fn get<T: DeserializeOwned>(url: String) -> T {
        debug!(url, "requesting from resource");
        ureq::get(&url)
            .call()
            .unwrap()
            .into_json()
            .unwrap()
    }

    // get the latest commit information from the `url`
    fn get_version(url: String) -> Option<Value> {
        debug!(url, "requesting version info for resource");

        match ureq::get(&url).call() {
            Ok(res) => Some(res.into_json::<Value>().unwrap()),
            Err(err) => {
                match err {
                    ureq::Error::Status(s, r) => {
                        warn!("{s} {} unable to check online version information", r.status_text());
                        None
                    }
                    ureq::Error::Transport(t) => {
                        if let Some(err) = t.message() {
                            warn!(%err, "something went wrong when trying to fetch online version information");
                        }
                        None
                    }
                }
            }
        }
    }

    // get the deserialized content of a local file at `path`
    fn get_file<T: DeserializeOwned>(path: PathBuf) -> T {
        let path_str = path.to_str().unwrap();
        debug!(path_str, "requesting from file");

        let mut file = File::options().read(true).open(path).unwrap();
        let mut content: Vec<u8> = Vec::new();
        file.read_to_end(&mut content).unwrap();

        serde_json::from_slice::<T>(&content).unwrap()
    }

    pub fn keys(&self) -> &HashMap<u32, Vec<u8>> {
        &self.keys
    }

    fn lookup_avatar_name(&self, avatar_id: u32) -> Option<String> {
        if avatar_id == 0 {
            return None;
        }

        // trailblazer
        if avatar_id >= 8000 {
            Some("Trailblazer".to_string())
        } else {
            let cfg = self.avatar_config.get(&avatar_id)?;
            cfg.AvatarName.lookup(&self.text_map).map(|s| s.to_string())
        }
    }
}

#[tracing::instrument(name = "relic", skip_all, fields(id = proto.tid))]
fn export_proto_relic(db: &Database, proto: &ProtoRelic) -> Option<Relic> {
    let relic_config = db.relic_config.get(&proto.tid)?;

    let set_config = db.relic_set_config.get(&relic_config.SetID)?;
    let main_affix_config = db.relic_main_affix_config.get(&relic_config.MainAffixGroup, &proto.main_affix_id).unwrap();

    let id = proto.unique_id.to_string();
    let level = proto.level;
    let lock = proto.is_protected;
    let discard = proto.is_discarded;
    let set = set_config.SetName.lookup(&db.text_map)
        .map(|s| s.to_string())
        .unwrap_or("".to_string());

    let slot = slot_type_to_export(&relic_config.Type);
    let rarity = relic_config.MaxLevel / 3;
    let mainstat = main_stat_to_export(&main_affix_config.Property).to_string();
    let location = db.lookup_avatar_name(proto.base_avatar_id).unwrap_or("".to_string());

    debug!(rarity, set, slot, slot, mainstat, location, "detected");

    let substats = proto.sub_affix_list.iter()
        .filter_map(|substat| export_substat(db, rarity, substat))
        .collect();


    Some(Relic {
        set,
        slot,
        rarity,
        level,
        mainstat,
        substats,
        location,
        lock,
        discard,
        _id: id,
    })
}

#[tracing::instrument(name = "substat", skip_all)]
fn export_substat(db: &Database, rarity: u32, substat: &RelicAffix) -> Option<Substat> {
    let cfg = db.relic_sub_affix_config.get(&rarity, &substat.affix_id)?;
    let key = sub_stat_to_export(&cfg.Property).to_string();

    let mut value = substat.cnt as f32 * *cfg.BaseValue
        + substat.step as f32 * *cfg.StepValue;

    if key.ends_with('_') {
        value *= 100.0;
    }

    trace!(key, value, "detected substat");

    Some(Substat {
        key,
        value,
    })
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Relic {
    pub set: String,
    pub slot: &'static str,
    pub rarity: u32,
    pub level: u32,
    pub mainstat: String,
    pub substats: Vec<Substat>,
    pub location: String,
    pub lock: bool,
    pub discard: bool,
    pub _id: String,
}

#[derive(Serialize, Deserialize, Debug)]
pub struct Substat {
    key: String,
    value: f32,
}

fn slot_type_to_export(s: &str) -> &'static str {
    match s {
        "HEAD" => "Head",
        "HAND" => "Hands",
        "BODY" => "Body",
        "FOOT" => "Feet",
        "NECK" => "Planar Sphere",
        "OBJECT" => "Link Rope",
        _ => panic!("Unknown slot: {}", s)
    }
}

fn main_stat_to_export(s: &str) -> &'static str {
    match s {
        "HPDelta" => "HP",
        "AttackDelta" => "ATK",
        "HPAddedRatio" => "HP",
        "AttackAddedRatio" => "ATK",
        "DefenceAddedRatio" => "DEF",
        "CriticalChanceBase" => "CRIT Rate",
        "CriticalDamageBase" => "CRIT DMG",
        "HealRatioBase" => "Outgoing Healing Boost",
        "SpeedDelta" => "SPD",
        "StatusProbabilityBase" => "Effect Hit Rate",
        "PhysicalAddedRatio" => "Physical DMG Boost",
        "FireAddedRatio" => "Fire DMG Boost",
        "IceAddedRatio" => "Ice DMG Boost",
        "ThunderAddedRatio" => "Lightning DMG Boost",
        "WindAddedRatio" => "Wind DMG Boost",
        "QuantumAddedRatio" => "Quantum DMG Boost",
        "ImaginaryAddedRatio" => "Imaginary DMG Boost",
        "BreakDamageAddedRatioBase" => "Break Effect",
        "SPRatioBase" => "Energy Regeneration Rate",
        _ => panic!("Unknown main stat: {}", s)
    }
}

fn sub_stat_to_export(s: &str) -> &'static str {
    match s {
        "HPDelta" => "HP",
        "AttackDelta" => "ATK",
        "HPAddedRatio" => "HP_",
        "AttackAddedRatio" => "ATK_",
        "DefenceAddedRatio" => "DEF_",
        "DefenceDelta" => "DEF",
        "CriticalChanceBase" => "CRIT Rate_",
        "CriticalDamageBase" => "CRIT DMG_",
        "SpeedDelta" => "SPD",
        "StatusProbabilityBase" => "Effect Hit Rate_",
        "StatusResistanceBase" => "Effect RES_",
        "BreakDamageAddedRatioBase" => "Break Effect_",
        _ => { panic!("Unknown sub stat: {}", s) }
    }
}

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

#[instrument(name = "light_cone", skip_all, fields(id = proto.tid))]
fn export_proto_light_cone(db: &Database, proto: &ProtoLightCone) -> Option<LightCone> {
    let cfg = db.equipment_config.get(&proto.tid)?;
    let key = cfg.EquipmentName.lookup(&db.text_map).map(|s| s.to_string())?;

    let level = proto.level;
    let superimposition = proto.rank;

    debug!(light_cone=key, level, superimposition, "detected");

    let location = db.lookup_avatar_name(proto.base_avatar_id)
        .unwrap_or("".to_string());

    Some(LightCone {
        key,
        level,
        ascension: proto.promotion,
        superimposition,
        location,
        lock: proto.is_protected,
        _id: proto.unique_id.to_string(),
    })
}

#[instrument(name = "character", skip_all, fields(id = proto.base_avatar_id))]
fn export_proto_character(db: &Database, proto: &ProtoCharacter) -> Option<Character> {
    let key = db.lookup_avatar_name(proto.base_avatar_id)?;

    let level = proto.level;
    let eidolon = proto.rank;

    debug!(character=key, level, eidolon, "detected");

    let (skills, traces) = export_skill_tree(db, &proto.skilltree_list);

    Some(Character {
        key,
        level,
        ascension: proto.promotion,
        eidolon,
        skills,
        traces,
    })
}

fn export_proto_hero(db: &Database, proto: &HeroBasicTypeInfo) -> Option<Character> {
    let path = avatar_path_lookup(db, proto.basic_type.value() as u32)?;
    let key = format!("Trailblazer{}", path);

    let span = info_span!("character", key);
    let _enter = span.enter();

    trace!(character=key, "detected");

    let (skills, traces) = export_skill_tree(db, &proto.skill_tree_list);

    // TODO: figure out where level/ascension is stored
    Some(Character {
        key,
        level: 0,
        ascension: 0,
        eidolon: proto.rank,
        skills,
        traces,
    })
}

fn avatar_path_lookup(db: &Database, avatar_id: u32) -> Option<&'static str> {
    let hero_config = db.avatar_config.get(&avatar_id);
    let avatar_base_type = hero_config.unwrap().AvatarBaseType.as_str();
    match avatar_base_type {
        "Knight"  => Some("Preservation"),
        "Rogue"   => Some("Hunt"),
        "Mage"    => Some("Erudition"),
        "Warlock" => Some("Nihility"),
        "Warrior" => Some("Destruction"),
        "Shaman"  => Some("Harmony"),
        "Priest"  => Some("Abundance"),
        _ => {
            debug!(?avatar_base_type, "unknown path");
            None
        }
    }
}

fn export_skill_tree(db: &Database, proto: &[ProtoSkillTree]) -> (Skills, Traces) {
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

    for skill in proto.iter().filter(|s| s.point_id != 0) {
        let level = skill.level;

        let span = info_span!("skill", id = skill.point_id, level);
        let _enter = span.enter();

        let Some(skill_tree_config) = db.avatar_skill_tree_config
            .get(&skill.point_id, &skill.level) else
        {
            warn!("could not look up skill tree config");
            continue;
        };

        match skill_tree_config.Anchor.as_str() {
            "Point01" => {
                trace!(level, "detected basic atk trace");
                skills.basic = level;
            }
            "Point02" => {
                trace!(level, "detected skill trace");
                skills.skill = level;
            }
            "Point03" => {
                trace!(level, "detected ult trace");
                skills.ult = level;
            }
            "Point04" => {
                trace!(level, "detected talent trace");
                skills.talent = level;
            }

            "Point05" => {
                trace!(level, "detected technique trace");
                /* technique */
            }

            "Point06" => {
                trace!("detected major trace 1");
                traces.ability_1 = true;
            }
            "Point07" => {
                trace!("detected major trace 2");
                traces.ability_2 = true;
            }
            "Point08" => {
                trace!("detected major trace 3");
                traces.ability_3 = true;
            }

            "Point09" => {
                trace!("detected minor trace 1");
                traces.stat_1 = true;
            }
            "Point10" => {
                trace!("detected minor trace 2");
                traces.stat_2 = true;
            }
            "Point11" => {
                trace!("detected minor trace 3");
                traces.stat_3 = true;
            }
            "Point12" => {
                trace!("detected minor trace 4");
                traces.stat_4 = true;
            }
            "Point13" => {
                trace!("detected minor trace 5");
                traces.stat_5 = true;
            }
            "Point14" => {
                trace!("detected minor trace 6");
                traces.stat_6 = true;
            }
            "Point15" => {
                trace!("detected minor trace 7");
                traces.stat_7 = true;
            }
            "Point16" => {
                trace!("detected minor trace 8");
                traces.stat_8 = true;
            }
            "Point17" => {
                trace!("detected minor trace 9");
                traces.stat_9 = true;
            }
            "Point18" => {
                trace!("detected minor trace 10");
                traces.stat_10 = true;
            }

            _ => {
                warn!(anchor = skill_tree_config.Anchor, "unknown point anchor");
                continue;
            }
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
