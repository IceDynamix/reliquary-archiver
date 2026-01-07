//! Windows startup registry integration.
//!
//! This module handles adding/removing the application from the Windows
//! startup registry (`HKEY_CURRENT_USER\SOFTWARE\Microsoft\Windows\CurrentVersion\Run`).
//!
//! When enabled, the application will automatically launch when Windows starts.

use std::env::current_exe;

use windows_registry::{Type, CURRENT_USER, HSTRING};

/// Errors that can occur during registry operations.
#[derive(Debug, Eq, PartialEq)]
pub enum RegistryError {
    /// Failed to create or open the registry key
    KeyCreationFailed,
    /// Could not determine the path to the current executable
    PathUnobtainable,
    /// Failed to add the registry value
    AddFailed,
    /// Failed to remove the registry value
    RemoveFailed,
}

/// Registry key name for the startup entry
const REGISTRY_KEY_NAME: &str = "reliquary-archiver";
/// Path to the Windows Run registry key
const REGISTRY_KEY_PATH: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";

/// Enables or disables the application's run-on-startup setting.
///
/// When enabled, adds the current executable path to the Windows startup registry.
/// When disabled, removes the registry entry.
///
/// # Errors
/// Returns a [`RegistryError`] if the registry operation fails.
pub fn set_run_on_start(enabled: bool) -> Result<(), RegistryError> {
    let key = CURRENT_USER
        .create(REGISTRY_KEY_PATH)
        .map_err(|e| RegistryError::KeyCreationFailed)?;

    let path_to_exe = current_exe().map_err(|e| RegistryError::PathUnobtainable)?;

    let operation_successful = if enabled {
        key.set_string(REGISTRY_KEY_NAME, path_to_exe.to_str().unwrap())
            .map_err(|e| RegistryError::AddFailed)
    } else {
        key.remove_value(REGISTRY_KEY_NAME).map_err(|e| RegistryError::RemoveFailed)
    };

    operation_successful
}

/// Checks if the registry state matches the expected setting.
///
/// This handles cases where the executable has been moved after enabling
/// run-on-start, ensuring the settings stay in sync with the actual registry.
///
/// # Returns
/// - `Ok(true)` if the registry matches the expected state
/// - `Ok(false)` if there's a mismatch (caller should update settings)
/// - `Err(RegistryError)` if the registry cannot be accessed
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
