use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::path::Path;

use reliquary::resource::excel::{
    AvatarConfigMap, AvatarExRankConfigMap, AvatarSkillTreeConfigMap, EquipmentConfigMap, ItemConfigMap, MultiplePathAvatarConfigMap,
    RelicConfigMap, RelicMainAffixConfigMap, RelicSetConfigMap, RelicSubAffixConfigMap,
};
use reliquary::resource::{ResourceMap, TextMapEntry};
use ureq::serde::Serialize;
use ureq::serde::de::DeserializeOwned;

const GITLAB_FILES_API_PATH: &str = "projects/Dimbreath%2Fturnbasedgamedata/repository/files";
const GITHUB_RAW_URL: &str = "https://raw.githubusercontent.com/DimbreathBot/TurnBasedGameData/main";

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
        download_config!($t, $ex, [&resource_path::<$t>()]);
    };

    ($t:ty) => {
        download_config!($t, |_| {}, [&resource_path::<$t>()]);
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
        download_config_and_store_text_hashes!($t, $field, $hashes, [&resource_path::<$t>()]);
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
        &resource_path::<AvatarConfigMap>(),
        &resource_path_of("AvatarConfigLD.json"),
    ]);
    download_config_and_store_text_hashes!(EquipmentConfigMap, EquipmentName, text_hashes);
    download_config_and_store_text_hashes!(RelicSetConfigMap, SetName, text_hashes);
    download_config_and_store_partial_text_hashes!(ItemConfigMap, ItemName, text_hashes);

    download_config!(AvatarSkillTreeConfigMap, [
        &resource_path::<AvatarSkillTreeConfigMap>(),
        &resource_path_of("AvatarSkillTreeConfigLD.json"),
    ]);
    download_config!(MultiplePathAvatarConfigMap);
    download_config!(RelicConfigMap);
    download_config!(RelicMainAffixConfigMap);
    download_config!(RelicSubAffixConfigMap);
    download_config!(AvatarExRankConfigMap);

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

    let text_map: HashMap<String, String> = download_as_json::<HashMap<String, String>>(&format!("TextMap/{file_name}"))
        .into_iter()
        .filter(|(k, _)| hashes.contains(k))
        .collect();

    write_to_out(text_map, &file_name);
}

fn resource_path<T: ResourceMap>() -> String {
    resource_path_of(T::get_json_name())
}

fn resource_path_of(name: &str) -> String {
    format!("ExcelOutput/{name}")
}

/// Downloads a file from the game data repo, given its path within the repo.
///
/// Tries GitLab through glab (authenticated, avoids rate limits), then GitLab
/// unauthenticated, then the GitHub mirror.
fn download_as_json<T: DeserializeOwned>(path: &str) -> T {
    let gitlab_api_path = format!("{GITLAB_FILES_API_PATH}/{}/raw?ref=main", path.replace('/', "%2F"));

    if let Some(value) = download_with_glab(&gitlab_api_path) {
        return value;
    }

    let urls = [
        format!("https://gitlab.com/api/v4/{gitlab_api_path}"),
        format!("{GITHUB_RAW_URL}/{path}"),
    ];
    let mut errors = Vec::new();
    for url in &urls {
        match download_with_ureq(url) {
            Ok(value) => return value,
            Err(e) => {
                println!("cargo:warning=failed to download {url}: {e}");
                errors.push(format!("{url}: {e}"));
            }
        }
    }

    panic!("Failed to download {path} from any source:\n{}", errors.join("\n"));
}

fn download_with_ureq<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    ureq::get(url)
        .call()
        .map_err(|e| e.to_string())?
        .into_json()
        .map_err(|e| format!("invalid json: {e}"))
}

fn download_with_glab<T: DeserializeOwned>(api_path: &str) -> Option<T> {
    if !glab_available() {
        return None;
    }

    // --hostname, since glab would otherwise use this repo's (GitHub) remote
    let output = match std::process::Command::new("glab")
        .args(["api", "--hostname", "gitlab.com", api_path])
        .output()
    {
        Ok(output) => output,
        Err(e) => {
            println!("cargo:warning=glab failed to run ({e}), downloading {api_path} unauthenticated");
            return None;
        }
    };

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        println!(
            "cargo:warning=glab api failed ({}), downloading {api_path} unauthenticated",
            stderr.trim()
        );
        return None;
    }

    match ureq::serde_json::from_slice(&output.stdout) {
        Ok(value) => Some(value),
        Err(e) => {
            println!("cargo:warning=glab returned invalid json ({e}), downloading {api_path} unauthenticated");
            None
        }
    }
}

fn glab_available() -> bool {
    static GLAB: std::sync::OnceLock<bool> = std::sync::OnceLock::new();

    *GLAB.get_or_init(|| {
        std::process::Command::new("glab")
            .arg("--version")
            .output()
            .is_ok_and(|o| o.status.success())
    })
}

fn write_to_out<T: DeserializeOwned + Serialize>(value: T, file_name: &str) {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join(file_name);

    let mut file = File::create(out_path).unwrap();

    ureq::serde_json::to_writer(&mut file, &value).unwrap();
}
