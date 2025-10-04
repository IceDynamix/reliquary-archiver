use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::path::Path;

use reliquary::resource::excel::{
    AvatarConfigMap, AvatarSkillTreeConfigMap, EquipmentConfigMap, ItemConfigMap, MultiplePathAvatarConfigMap, RelicConfigMap,
    RelicMainAffixConfigMap, RelicSetConfigMap, RelicSubAffixConfigMap,
};
use reliquary::resource::{ResourceMap, TextMapEntry};
use ureq::serde::de::DeserializeOwned;
use ureq::serde::Serialize;
use ureq::serde_json::Value;

const BASE_RESOURCE_URL: &str = "https://gitlab.com/Dimbreath/turnbasedgamedata/-/raw/main";
const KEY_URL: &str = "https://raw.githubusercontent.com/tamilpp25/Iridium-SR/refs/heads/main/data/Keys.json";

macro_rules! download_config {
    ($t:ty, $ex:expr, [$($url:expr),+ $(,)?]) => {
        write_to_out(
            {
                let mut merged = <$t>::new_empty();
                $(
                    let value = download_as_json::<$t>($url);
                    $ex(&value);
                    merged.0.extend(value.0);
                )+
                merged
            },
            <$t>::get_json_name(),
        )
    };

    ($t:ty, [$($url:expr),+ $(,)?]) => {
        download_config!($t, |_| {}, [$($url),+]);
    };

    ($t:ty, $ex:expr) => {
        download_config!($t, $ex, [&resource_url::<$t>()]);
    };

    ($t:ty) => {
        download_config!($t, |_| {}, [&resource_url::<$t>()]);
    };
}

macro_rules! download_config_and_store_text_hashes {
    ($t:ty, $field:ident, $hashes:ident, $urls:tt) => {
        download_config!(
            $t,
            |value: &$t| {
                for cfg in value.0.iter() {
                    $hashes.insert(cfg.$field);
                }
            },
            $urls
        );
    };

    ($t:ty, $field:ident, $hashes:ident) => {
        download_config_and_store_text_hashes!($t, $field, $hashes, [&resource_url::<$t>()]);
    };
}

macro_rules! download_config_and_store_partial_text_hashes {
    ($t:ty, $field:ident, $hashes:ident) => {
        download_config!($t, |value: &$t| {
            for cfg in value.0.iter() {
                if let Some(hash) = cfg.$field {
                    $hashes.insert(hash);
                }
            }
        });
    };
}

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");

    // the text map is really, REALLY large (>25MB), so we're optimizing by only
    // keeping the entries used from relevant config files where the strings are required
    // for the export
    let mut text_hashes: HashSet<TextMapEntry> = HashSet::new();

    download_config_and_store_text_hashes!(
        AvatarConfigMap,
        AvatarName,
        text_hashes,
        [&resource_url::<AvatarConfigMap>(), &resource_url_of("AvatarConfigLD.json"),]
    );
    download_config_and_store_text_hashes!(EquipmentConfigMap, EquipmentName, text_hashes);
    download_config_and_store_text_hashes!(RelicSetConfigMap, SetName, text_hashes);
    download_config_and_store_partial_text_hashes!(ItemConfigMap, ItemName, text_hashes);

    download_config!(
        AvatarSkillTreeConfigMap,
        [
            &resource_url::<AvatarSkillTreeConfigMap>(),
            &resource_url_of("AvatarSkillTreeConfigLD.json"),
        ]
    );
    download_config!(MultiplePathAvatarConfigMap);
    download_config!(RelicConfigMap);
    download_config!(RelicMainAffixConfigMap);
    download_config!(RelicSubAffixConfigMap);

    save_text_map(&text_hashes, "EN");

    write_to_out(download_as_json::<Value>(KEY_URL), "keys.json");

    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico").set("InternalName", "Reliquary Archiver");
        res.compile().unwrap();
    }
}

fn save_text_map(hashes: &HashSet<TextMapEntry>, language: &str) {
    let hashes: HashSet<String> = hashes.iter().map(|k| k.Hash.to_string()).collect();

    let file_name = format!("TextMap{language}.json");

    let text_map_url = format!("{BASE_RESOURCE_URL}/TextMap/{file_name}");
    let text_map: HashMap<String, String> = download_as_json::<HashMap<String, String>>(&text_map_url)
        .into_iter()
        .filter(|(k, _)| hashes.contains(k))
        .collect();

    write_to_out(text_map, &file_name);
}

fn resource_url<T: ResourceMap>() -> String {
    resource_url_of(T::get_json_name())
}

fn resource_url_of(name: &str) -> String {
    format!("{BASE_RESOURCE_URL}/ExcelOutput/{name}")
}

fn download_as_json<T: DeserializeOwned>(url: &str) -> T {
    ureq::get(url)
        .call()
        .unwrap()
        .into_json()
        .expect(format!("Failed to read json from url: {}", url).as_str())
}

fn write_to_out<T: DeserializeOwned + Serialize>(value: T, file_name: &str) {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join(file_name);

    let mut file = File::create(out_path).unwrap();

    ureq::serde_json::to_writer(&mut file, &value).unwrap();
}
