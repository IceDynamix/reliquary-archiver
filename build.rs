use std::collections::{HashMap, HashSet};
use std::env;
use std::fs::File;
use std::path::Path;

use reliquary::resource::excel::{
    AvatarConfig, AvatarConfigMap, AvatarExRankConfig, AvatarExRankConfigMap, AvatarSkillTreeConfig, AvatarSkillTreeConfigMap,
    EquipmentConfig, EquipmentConfigMap, ItemConfig, ItemConfigMap, MultiplePathAvatarConfig, MultiplePathAvatarConfigMap, RelicConfig,
    RelicConfigMap, RelicMainAffixConfig, RelicMainAffixConfigMap, RelicSetConfig, RelicSetConfigMap, RelicSubAffixConfig,
    RelicSubAffixConfigMap,
};
use reliquary::resource::{ResourceMap, TextMapEntry};
use ureq::serde::Serialize;
use ureq::serde::de::DeserializeOwned;

const GITLAB_FILES_API_PATH: &str = "projects/Dimbreath%2Fturnbasedgamedata/repository/files";
const GITHUB_RAW_URL: &str = "https://raw.githubusercontent.com/DimbreathBot/TurnBasedGameData/main";

fn main() {
    println!("cargo:rerun-if-changed=Cargo.toml");

    // `pktmon_only`: pktmon is the capture backend, pcap is preferred when both are built in
    println!("cargo::rustc-check-cfg=cfg(pktmon_only)");
    if env::var_os("CARGO_FEATURE_PKTMON").is_some() && env::var_os("CARGO_FEATURE_PCAP").is_none() {
        println!("cargo::rustc-cfg=pktmon_only");
    }

    // the text map is really, REALLY large (>25MB), so we're optimizing by only
    // keeping the entries used from relevant config files where the strings are required
    // for the export
    let mut text_hashes: HashSet<TextMapEntry> = HashSet::new();

    let avatars = download_config::<AvatarConfigMap, AvatarConfig>(&["AvatarConfigLD.json"]);
    text_hashes.extend(avatars.iter().map(|cfg| cfg.AvatarName));
    let equipment = download_config::<EquipmentConfigMap, EquipmentConfig>(&[]);
    text_hashes.extend(equipment.iter().map(|cfg| cfg.EquipmentName));
    let relic_sets = download_config::<RelicSetConfigMap, RelicSetConfig>(&[]);
    text_hashes.extend(relic_sets.iter().map(|cfg| cfg.SetName));
    let items = download_config::<ItemConfigMap, ItemConfig>(&[]);
    text_hashes.extend(items.iter().filter_map(|cfg| cfg.ItemName));

    download_config::<AvatarSkillTreeConfigMap, AvatarSkillTreeConfig>(&["AvatarSkillTreeConfigLD.json"]);
    download_config::<MultiplePathAvatarConfigMap, MultiplePathAvatarConfig>(&[]);
    download_config::<RelicConfigMap, RelicConfig>(&[]);
    download_config::<RelicMainAffixConfigMap, RelicMainAffixConfig>(&[]);
    download_config::<RelicSubAffixConfigMap, RelicSubAffixConfig>(&[]);
    download_config::<AvatarExRankConfigMap, AvatarExRankConfig>(&[]);

    save_text_map(&text_hashes, "EN");

    #[cfg(target_os = "windows")]
    {
        let mut res = winres::WindowsResource::new();
        res.set_icon("assets/icon.ico").set("InternalName", "Reliquary Archiver");
        res.compile().unwrap();
    }
}

/// Downloads the entries of config map `M`, plus those in `more_files` (configs some entries are
/// kept apart in), and writes them all to OUT_DIR as `M`'s file. Returns the entries.
fn download_config<M: ResourceMap, Entry: Serialize + DeserializeOwned>(more_files: &[&str]) -> Vec<Entry> {
    let files = std::iter::once(M::get_json_name()).chain(more_files.iter().copied());
    let entries: Vec<Entry> = files
        .flat_map(|file| download_as_json::<Vec<Entry>>(&resource_path(file)))
        .collect();
    write_to_out(&entries, M::get_json_name());
    entries
}

fn save_text_map(hashes: &HashSet<TextMapEntry>, language: &str) {
    let hashes: HashSet<String> = hashes.iter().map(|k| k.Hash.to_string()).collect();

    let file_name = format!("TextMap{language}.json");

    let text_map: HashMap<String, String> = download_as_json::<HashMap<String, String>>(&format!("TextMap/{file_name}"))
        .into_iter()
        .filter(|(k, _)| hashes.contains(k))
        .collect();

    write_to_out(&text_map, &file_name);
}

fn resource_path(name: &str) -> String {
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

fn write_to_out<T: Serialize>(value: &T, file_name: &str) {
    let out_dir = env::var_os("OUT_DIR").unwrap();
    let out_path = Path::new(&out_dir).join(file_name);

    let mut file = File::create(out_path).unwrap();

    ureq::serde_json::to_writer(&mut file, value).unwrap();
}
