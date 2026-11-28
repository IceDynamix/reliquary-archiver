//! Where the app keeps its files, and writing them safely.

use std::io;
use std::path::{Path, PathBuf};

const APP: &str = "reliquary-archiver";

fn home(rest: &[&str]) -> Option<PathBuf> {
    let mut path = PathBuf::from(std::env::var_os("HOME")?);
    path.extend(rest);
    Some(path)
}

/// An XDG folder from its variable, or its default under the home folder
fn xdg(var: &str, default: &[&str]) -> Option<PathBuf> {
    std::env::var_os(var)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .or_else(|| home(default))
}

fn local_app_data() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(PathBuf::from)
}

fn absolute(path: Option<PathBuf>) -> Option<PathBuf> {
    path.filter(|path| path.is_absolute())
}

/// The settings: the local app data folder on Windows, Application Support on macOS, the XDG
/// config folder elsewhere
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn config_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        local_app_data()
    } else if cfg!(target_os = "macos") {
        home(&["Library", "Application Support"])
    } else {
        xdg("XDG_CONFIG_HOME", &[".config"])
    };
    absolute(base).map(|path| path.join(APP))
}

/// What can be fetched or made again (downloads, scaled backgrounds): under the local app data
/// folder on Windows, Caches on macOS, the XDG cache folder elsewhere
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn cache_dir() -> Option<PathBuf> {
    let base = if cfg!(windows) {
        local_app_data().map(|dir| dir.join(APP).join("Cache"))
    } else if cfg!(target_os = "macos") {
        home(&["Library", "Caches", APP])
    } else {
        xdg("XDG_CACHE_HOME", &[".cache"]).map(|dir| dir.join(APP))
    };
    absolute(base)
}

/// Writes the file beside where it goes and moves it in place, so a crash halfway never leaves a
/// half written file to be read: it's on disk before it's moved, so a power cut doesn't either. The
/// temporary name has the process id, so two running copies don't write to the same one
#[cfg_attr(not(feature = "gui"), allow(dead_code))]
pub fn write_atomic(path: &Path, write: impl FnOnce(&Path) -> io::Result<()>) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temp = path.as_os_str().to_owned();
    temp.push(format!(".{}.tmp", std::process::id()));
    let temp = PathBuf::from(temp);
    let written = write(&temp)
        .and_then(|()| std::fs::File::open(&temp)?.sync_all())
        .and_then(|()| std::fs::rename(&temp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    written
}
