//! Starting the app on login, through the current user's `Run` key
//! (`HKEY_CURRENT_USER\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`).

use std::env::current_exe;
use std::fmt;

use windows_registry::{CURRENT_USER, HSTRING, Key, Type};

/// What went wrong reading or changing the `Run` key
#[derive(Debug)]
pub enum RegistryError {
    /// The `Run` key couldn't be opened
    OpenKey(windows::core::Error),
    /// The path to this executable couldn't be found
    ExePath(std::io::Error),
    /// The entry couldn't be written
    Add(windows::core::Error),
    /// The entry couldn't be removed
    Remove(windows::core::Error),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::OpenKey(e) => write!(f, "Couldn't open the startup programs list: {e}"),
            Self::ExePath(e) => write!(f, "Couldn't find where the app is: {e}"),
            Self::Add(e) => write!(f, "Couldn't add the app to the startup programs: {e}"),
            Self::Remove(e) => write!(f, "Couldn't remove the app from the startup programs: {e}"),
        }
    }
}

impl std::error::Error for RegistryError {}

/// The value's name under the `Run` key
const REGISTRY_KEY_NAME: &str = "reliquary-archiver";
const REGISTRY_KEY_PATH: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";

fn run_key() -> Result<Key, RegistryError> {
    CURRENT_USER.create(REGISTRY_KEY_PATH).map_err(RegistryError::OpenKey)
}

/// Adds this executable to the programs started on login, or removes it
pub fn set_run_on_start(enabled: bool) -> Result<(), RegistryError> {
    let key = run_key()?;
    if enabled {
        let exe = current_exe().map_err(RegistryError::ExePath)?;
        key.set_hstring(REGISTRY_KEY_NAME, &HSTRING::from(exe.as_os_str()))
            .map_err(RegistryError::Add)
    } else {
        key.remove_value(REGISTRY_KEY_NAME).map_err(RegistryError::Remove)
    }
}

/// Brings the `Run` entry in line with this executable, and says whether run-on-start is on.
///
/// An entry that isn't a string, or names another path (the executable was moved since), is
/// removed, so run-on-start counts as off.
pub fn reconcile_registry() -> Result<bool, RegistryError> {
    let key = run_key()?;
    let Ok(value) = key.get_value(REGISTRY_KEY_NAME) else {
        return Ok(false);
    };

    let registered = (value.ty() == Type::String).then(|| {
        let path = HSTRING::from_wide(value.as_wide()).to_string_lossy();
        path.trim_end_matches('\0').to_owned()
    });
    let exe = current_exe().map_err(RegistryError::ExePath)?;
    if registered.as_deref().is_some_and(|path| std::path::Path::new(path) == exe) {
        return Ok(true);
    }

    tracing::info!(?registered, exe = %exe.display(), "the startup entry is stale, removing it");
    key.remove_value(REGISTRY_KEY_NAME).map_err(RegistryError::Remove)?;
    Ok(false)
}
