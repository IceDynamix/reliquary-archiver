//! Self-update functionality for Windows builds

use std::env;
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use self_update::cargo_crate_version;
use tracing::info;

/// Flag to indicate that the updated version should be spawned after GUI exits
pub static SPAWN_UPDATED_VERSION: AtomicBool = AtomicBool::new(false);

/// Set the flag to spawn updated version after GUI exits
pub fn request_spawn_after_exit() {
    SPAWN_UPDATED_VERSION.store(true, Ordering::SeqCst);
}

/// Check if we should spawn the updated version
pub fn should_spawn_after_exit() -> bool {
    SPAWN_UPDATED_VERSION.load(Ordering::SeqCst)
}

const REPO_OWNER: &str = "IceDynamix";
const REPO_NAME: &str = "reliquary-archiver";
const TARGET: &str = "x64";

fn identifier() -> Option<&'static str> {
    if cfg!(feature = "pcap") {
        Some("pcap")
    } else if cfg!(feature = "pktmon") {
        Some("pktmon")
    } else {
        None
    }
}

fn configure_builder() -> self_update::backends::github::UpdateBuilder {
    let mut builder = self_update::backends::github::Update::configure();
    builder
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .bin_name(REPO_NAME)
        .target(TARGET)
        .current_version(cargo_crate_version!());
    if let Some(id) = identifier() {
        builder.identifier(id);
    }
    builder
}

/// Interactive update for CLI mode (shows download progress, prompts user)
/// Spawns updated version after GUI exits if update is successful
pub fn update_interactive(auth_token: Option<&str>, no_confirm: bool) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    info!("checking for updates");

    let mut update_builder = configure_builder();
    update_builder
        .show_download_progress(true)
        .show_output(false)
        .no_confirm(no_confirm);

    if let Some(token) = auth_token {
        update_builder.auth_token(token);
    }

    let status = update_builder.build()?.update()?;

    if status.updated() {
        info!("updated to {}", status.version());
        spawn_updated_version()?;
    } else {
        info!("already up-to-date");
    }

    Ok(())
}

/// Perform the update (should be called after user confirms)
/// Returns true if the update was successful and the app should restart
pub fn update_noninteractive() -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
    let mut update_builder = configure_builder();
    update_builder
        .show_download_progress(false)
        .show_output(false)
        .no_confirm(true);

    let status = update_builder.build()?.update()?;

    if status.updated() {
        info!("updated to {}", status.version());
        return Ok(true);
    }

    Ok(false)
}

/// Information about an available update
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub download_url: String,
}

/// Check if an update is available without prompting or downloading
pub fn check_for_update() -> Result<Option<UpdateInfo>, Box<dyn std::error::Error + Send + Sync>> {
    use self_update::backends::github::ReleaseList;
    
    let releases = ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()?
        .fetch()?;

    let identifier = identifier();
    let current_version = cargo_crate_version!();

    let latest_release = releases.iter().find(|r| {
        r.asset_for(TARGET, identifier).is_some()
    });

    let latest_release = match latest_release {
        Some(r) => r,
        None => return Ok(None),
    };

    let latest_version = &latest_release.version;
    if !self_update::version::bump_is_greater(current_version, latest_version)? {
        return Ok(None);
    }

    let asset = latest_release.asset_for(TARGET, identifier).unwrap();

    Ok(Some(UpdateInfo {
        current_version: current_version.to_string(),
        latest_version: latest_version.clone(),
        download_url: asset.download_url.clone(),
    }))
}

/// Spawn the new version after update and exit
pub fn spawn_updated_version() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let current_exe = env::current_exe()?;
    let mut command = Command::new(current_exe);
    command.args(env::args().skip(1)).env("NO_SELF_UPDATE", "1");

    command.spawn().and_then(|mut c| c.wait())?;

    std::process::exit(0);
}
