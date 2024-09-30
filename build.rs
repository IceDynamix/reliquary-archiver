use std::env;
use std::fs::File;
use std::path::Path;

use reliquary::resource::excel::{
    AvatarConfigMap, AvatarSkillTreeConfigMap, EquipmentConfigMap, MultiplePathAvatarConfigMap,
    RelicConfigMap, RelicMainAffixConfigMap, RelicSetConfigMap, RelicSubAffixConfigMap,
};
use reliquary::resource::ResourceMap;
use ureq::serde_json::Value;

const BASE_RESOURCE_URL: &str = "https://raw.githubusercontent.com/Dimbreath/StarRailData/master";
const KEY_URL: &str =
    "https://raw.githubusercontent.com/juliuskreutz/stardb-exporter/master/hsr_keys.json";

fn main() {
    println!("cargo::rerun-if-changed=Cargo.toml");
    println!("cargo::rerun-if-changed=Cargo.lock");

    download_config::<AvatarConfigMap>();
    download_config::<AvatarSkillTreeConfigMap>();
    download_config::<EquipmentConfigMap>();
    download_config::<MultiplePathAvatarConfigMap>();
    download_config::<RelicConfigMap>();
    download_config::<RelicMainAffixConfigMap>();
    download_config::<RelicSetConfigMap>();
    download_config::<RelicSubAffixConfigMap>();

    download_and_write_to_out(
        "TextMapEN.json",
        format!("{BASE_RESOURCE_URL}/TextMap/TextMapEN.json").as_str(),
    );
    download_and_write_to_out("keys.json", KEY_URL);
}

fn download_config<T: ResourceMap>() {
    let file_name = T::get_json_name();

    let url = format!("{BASE_RESOURCE_URL}/ExcelOutput/{file_name}");

    download_and_write_to_out(file_name, &url);
}

fn download_and_write_to_out(file: &str, url: &str) {
    // downloaded files are in pretty format, deserialize and serialize
    // to compress file size
    let value: Value = ureq::get(url).call().unwrap().into_json().unwrap();

    let out_dir = env::var_os("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join(file);

    let mut file = File::create(out_path).unwrap();

    ureq::serde_json::to_writer(&mut file, &value).unwrap();
}
