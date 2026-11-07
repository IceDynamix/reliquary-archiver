use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::path::Path;

use reliquary::resource::excel::{
    AvatarConfigMap, AvatarSkillTreeConfigMap, EquipmentConfigMap, ItemConfigMap, MultiplePathAvatarConfigMap, RelicConfigMap,
    RelicMainAffixConfigMap, RelicSetConfigMap, RelicSubAffixConfigMap,
};
use reliquary::resource::{ResourceMap, TextMapEntry};
use ureq::serde::Serialize;
use ureq::serde::de::DeserializeOwned;

const API_RESOURCE_URL: &str = "https://gitlab.com/api/v4/projects/Dimbreath%2Fturnbasedgamedata/repository/files";

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

    download_config_and_store_text_hashes!(AvatarConfigMap, AvatarName, text_hashes, [
        &resource_url::<AvatarConfigMap>(),
        &resource_url_of("AvatarConfigLD.json"),
    ]);
    download_config_and_store_text_hashes!(EquipmentConfigMap, EquipmentName, text_hashes);
    download_config_and_store_text_hashes!(RelicSetConfigMap, SetName, text_hashes);
    download_config_and_store_partial_text_hashes!(ItemConfigMap, ItemName, text_hashes);

    download_config!(AvatarSkillTreeConfigMap, [
        &resource_url::<AvatarSkillTreeConfigMap>(),
        &resource_url_of("AvatarSkillTreeConfigLD.json"),
    ]);
    download_config!(MultiplePathAvatarConfigMap);
    download_config!(RelicConfigMap);
    download_config!(RelicMainAffixConfigMap);
    download_config!(RelicSubAffixConfigMap);

    save_text_map(&text_hashes, "EN");

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

    let encoded_path = format!("TextMap%2F{file_name}");
    let text_map_url = format!("{API_RESOURCE_URL}/{encoded_path}/raw?ref=main");
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
    let encoded_path = format!("ExcelOutput%2F{name}");
    format!("{API_RESOURCE_URL}/{encoded_path}/raw?ref=main")
}

fn download_as_json<T: DeserializeOwned>(url: &str) -> T {
    // Try glab CLI first (authenticated, avoids rate limits), fall back to ureq
    if let Some(data) = download_with_glab(url) {
        return ureq::serde_json::from_str(&data)
            .unwrap_or_else(|e| panic!("Failed to parse json from {url}: {e}"));
    }

    ureq::get(url)
        .call()
        .unwrap_or_else(|e| panic!("Failed to download {url}: {e}"))
        .into_json()
        .unwrap_or_else(|_| panic!("Failed to read json from url: {}", url))
}

fn download_with_glab(url: &str) -> Option<String> {
    let api_path = url.strip_prefix("https://gitlab.com/api/v4/")?;
    let glab = glab_path()?;

    // --hostname, since glab would otherwise use this repo's (GitHub) remote
    let output = match std::process::Command::new(glab).args(["api", "--hostname", "gitlab.com", api_path]).output() {
        Ok(output) => output,
        Err(e) => {
            println!("cargo:warning=glab failed to run ({e}), downloading {url} unauthenticated");
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!("cargo:warning=glab api failed ({}), downloading {url} unauthenticated", stderr.trim());
        return None;
    }

    String::from_utf8(output.stdout).ok()
}

fn glab_path() -> Option<&'static std::path::Path> {
    static GLAB: std::sync::OnceLock<Option<std::path::PathBuf>> = std::sync::OnceLock::new();

    GLAB.get_or_init(|| {
        let on_path = std::process::Command::new("glab")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success());
        if on_path {
            return Some("glab".into());
        }

        // winget installs here, and it isn't on PATH until the shell restarts
        let local = std::env::var("LOCALAPPDATA").ok()?;
        let path = std::path::PathBuf::from(local).join("Programs").join("glab").join("glab.exe");
        path.exists().then_some(path)
    })
    .as_deref()
}

fn write_to_out<T: DeserializeOwned + Serialize>(value: T, file_name: &str) {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join(file_name);

    let mut file = File::create(out_path).unwrap();

    ureq::serde_json::to_writer(&mut file, &value).unwrap();
}
