//! Where an authkey for the warp history comes from, besides the capture: a link to the history
//! the user pastes, or the one the game's own browser last opened, which it keeps in its cache.

use std::path::{Path, PathBuf};

use super::api::HistoryKey;

/// Reads the authkey and server out of a link to the warp history
pub fn from_url(url: &str) -> Option<HistoryKey> {
    let url = url.trim();
    let rest = url.strip_prefix("https://").or_else(|| url.strip_prefix("http://"))?;
    let (host, rest) = rest.split_once('/')?;
    let query = rest.split_once('?')?.1;
    let query = query.split('#').next().unwrap_or(query);

    let mut authkey = None;
    let mut game_biz = None;
    for pair in query.split('&') {
        let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
        match name {
            "authkey" => authkey = Some(percent_decode(value)?),
            "game_biz" => game_biz = Some(percent_decode(value)?),
            _ => {}
        }
    }
    let authkey = authkey.filter(|key| !key.is_empty())?;
    // The page itself lives elsewhere than its data, the data is what's asked for
    let host = if host.contains("webstatic") {
        if host.ends_with("mihoyo.com") {
            "public-operation-hkrpg.mihoyo.com".to_string()
        } else {
            "public-operation-hkrpg-sg.hoyoverse.com".to_string()
        }
    } else {
        host.to_string()
    };
    let game_biz = game_biz.unwrap_or_else(|| if host.ends_with("mihoyo.com") { "hkrpg_cn" } else { "hkrpg_global" }.to_string());
    Some(HistoryKey { authkey, host, game_biz })
}

fn percent_decode(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' => {
                let hex = std::str::from_utf8(bytes.get(i + 1..i + 3)?).ok()?;
                out.push(u8::from_str_radix(hex, 16).ok()?);
                i += 3;
            }
            b'+' => {
                out.push(b' ');
                i += 1;
            }
            byte => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8(out).ok()
}

/// The warp history link the game's browser opened last, from its cache. The game writes where
/// it's installed into its log, and keeps one cache folder per version of its browser
pub fn from_game_cache() -> Result<HistoryKey, String> {
    let data_dir = game_data_dirs()
        .into_iter()
        .next()
        .ok_or("Couldn't find where the game is installed. Start it once, then try again.")?;
    let caches = std::fs::read_dir(data_dir.join("webCaches")).map_err(|_| "The game hasn't opened any web pages yet.".to_string())?;
    let mut cache_files: Vec<(std::time::SystemTime, PathBuf)> = caches
        .flatten()
        .map(|entry| entry.path().join("Cache").join("Cache_Data").join("data_2"))
        .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
        .collect();
    cache_files.sort();

    for (_, path) in cache_files.iter().rev() {
        if let Some(key) = std::fs::read(path).ok().and_then(|bytes| last_history_url(&bytes)) {
            return Ok(key);
        }
    }
    Err("The game hasn't opened its warp history lately. Open Warp → View Details in the game, then try again.".into())
}

/// The last link to the warp history in a cache file
fn last_history_url(bytes: &[u8]) -> Option<HistoryKey> {
    const MARK: &[u8] = b"/hkrpg_gacha_record/api/get";
    let mut found = None;
    let mut from = 0;
    while let Some(at) = find(&bytes[from..], MARK).map(|at| at + from) {
        // Back to where the link starts, forward to where it ends
        let Some(start) = bytes[..at].windows(8).rposition(|w| w == b"https://") else {
            from = at + MARK.len();
            continue;
        };
        let end = bytes[at..]
            .iter()
            .position(|&b| b == 0 || b.is_ascii_whitespace() || !b.is_ascii())
            .map_or(bytes.len(), |end| at + end);
        if let Ok(url) = std::str::from_utf8(&bytes[start..end])
            && let Some(key) = from_url(url)
        {
            found = Some(key);
        }
        from = at + MARK.len();
    }
    found
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// The game's `StarRail_Data` folders, from the logs it writes on every start, newest log first
fn game_data_dirs() -> Vec<PathBuf> {
    let Some(profile) = std::env::var_os("USERPROFILE").map(PathBuf::from) else {
        return Vec::new();
    };
    let local_low = profile.join("AppData").join("LocalLow");
    let mut logs: Vec<(std::time::SystemTime, PathBuf)> = [
        local_low.join("Cognosphere").join("Star Rail"),
        local_low.join("miHoYo").join("崩坏：星穹铁道"),
    ]
    .iter()
    .flat_map(|dir| [dir.join("Player.log"), dir.join("Player-prev.log")])
    .filter_map(|path| Some((path.metadata().ok()?.modified().ok()?, path)))
    .collect();
    logs.sort();
    logs.iter().rev().filter_map(|(_, log)| data_dir_from_log(log)).collect()
}

fn data_dir_from_log(log: &Path) -> Option<PathBuf> {
    let text = std::fs::read(log).ok()?;
    let text = String::from_utf8_lossy(&text[..text.len().min(64 * 1024)]);
    let line = text.lines().find(|line| line.starts_with("Loading player data from "))?;
    let data_file = line.trim_start_matches("Loading player data from ").trim();
    let dir = Path::new(data_file).parent()?.to_path_buf();
    dir.is_dir().then_some(dir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_history_link_gives_its_key_and_server() {
        let url = "https://public-operation-hkrpg-sg.hoyoverse.com/common/hkrpg_gacha_record/api/getGachaLog?authkey_ver=1&sign_type=2&lang=en&authkey=ab%2bcd%2fef%3d&game_biz=hkrpg_global";
        let key = from_url(url).unwrap();
        assert_eq!(key.authkey, "ab+cd/ef=");
        assert_eq!(key.host, "public-operation-hkrpg-sg.hoyoverse.com");
        assert_eq!(key.game_biz, "hkrpg_global");
    }

    #[test]
    fn the_page_link_points_at_its_data() {
        let url = "https://webstatic.mihoyo.com/hkrpg/event/e20211215gacha-v2/index.html?authkey=x#/log";
        let key = from_url(url).unwrap();
        assert_eq!(key.host, "public-operation-hkrpg.mihoyo.com");
        assert_eq!(key.game_biz, "hkrpg_cn");
    }

    #[test]
    fn links_without_a_key_are_refused() {
        assert!(from_url("https://example.com/?lang=en").is_none());
        assert!(from_url("not a link").is_none());
    }

    #[test]
    fn the_last_link_in_a_cache_file_wins() {
        let mut bytes = b"\0\0https://public-operation-hkrpg-sg.hoyoverse.com/common/hkrpg_gacha_record/api/getGachaLog?authkey=old&game_biz=hkrpg_global\0junk".to_vec();
        bytes.extend_from_slice(b"\x01\x02https://public-operation-hkrpg-sg.hoyoverse.com/common/hkrpg_gacha_record/api/getGachaLog?authkey=new&game_biz=hkrpg_global\0");
        assert_eq!(last_history_url(&bytes).unwrap().authkey, "new");
    }
}
