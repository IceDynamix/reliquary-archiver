use windows_registry::{CURRENT_USER, HSTRING, Type};
use std::env::current_exe;

#[derive(Debug, Eq, PartialEq)]
pub enum RegistryError {
    KeyCreationFailed,
    PathUnobtainable,
    AddFailed,
    RemoveFailed,
}

const REGISTRY_KEY_NAME: &str = "reliquary-archiver";
const REGISTRY_KEY_PATH: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";

pub fn set_run_on_start(enabled: bool) -> Result<(), RegistryError> {
    let key = CURRENT_USER.create(REGISTRY_KEY_PATH).map_err(|e| { RegistryError::KeyCreationFailed })?;

    let path_to_exe = current_exe().map_err(|e| { RegistryError::PathUnobtainable })?;

    let operation_successful = if enabled {
        key.set_string(REGISTRY_KEY_NAME, path_to_exe.to_str().unwrap())
            .map_err(|e| { RegistryError::AddFailed })
    } else {
        key.remove_value(REGISTRY_KEY_NAME).map_err(|e| { RegistryError::RemoveFailed })
    };

    operation_successful
}

pub fn registry_matches_settings(enabled: bool) -> Result<bool, RegistryError> {
    let key = CURRENT_USER.create(REGISTRY_KEY_PATH);
    if key.is_err() {
        tracing::error!("Failed to get registry key handle");
        return Err(RegistryError::KeyCreationFailed);
    };
    let key = key.unwrap();

    let val = key.get_value(REGISTRY_KEY_NAME);
    if val.is_err() {
        if enabled {
            tracing::warn!("Run on start enabled but no key set in registry! Disabling setting.");
            return Ok(false);
        } else {
            return Ok(true);
        }
    }
    let val = val.unwrap();

    if val.ty() != Type::String {
        tracing::error!("Key in registry set but is not of type string! Removing registry key");
        key.remove_value(REGISTRY_KEY_NAME).unwrap();
        return Ok(false);
    }

    let path_to_exe = current_exe().unwrap().to_str().unwrap().to_owned() + "\0";
    let path_in_registry = HSTRING::from_wide(val.as_wide()).to_string_lossy();

    let paths_match = path_to_exe == path_in_registry;

    tracing::debug!("\npaths are:\n(exe): {:?}\nand\n(reg): {:?}", path_to_exe, path_in_registry);

    let matches = enabled == paths_match;

    if (!paths_match) {
        tracing::info!("Path in registry does not match path to exe. Removing registry key");
        key.remove_value(REGISTRY_KEY_NAME).unwrap();
    }
    Ok(matches)
}    