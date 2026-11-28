//! Self-update functionality for Windows builds

use std::env;
use std::process::Command;
#[cfg(feature = "gui")]
use std::sync::atomic::{AtomicBool, Ordering};

use self_update::cargo_crate_version;
use tracing::info;

/// Flag to indicate that the updated version should be spawned after GUI exits
#[cfg(feature = "gui")]
static SPAWN_UPDATED_VERSION: AtomicBool = AtomicBool::new(false);

/// Set the flag to spawn updated version after GUI exits
#[cfg(feature = "gui")]
pub fn request_spawn_after_exit() {
    SPAWN_UPDATED_VERSION.store(true, Ordering::SeqCst);
}

/// Check if we should spawn the updated version
#[cfg(feature = "gui")]
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

type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Installs the latest release if it's newer. Returns whether it did
fn install(builder: &self_update::backends::github::UpdateBuilder) -> Result<bool, BoxError> {
    let status = builder.build()?.update()?;
    if status.updated() {
        info!("updated to {}", status.version());
    }
    Ok(status.updated())
}

/// Updates from the command line, showing the download and asking first unless `no_confirm`,
/// then restarts into the new version
pub fn update_interactive(auth_token: Option<&str>, no_confirm: bool) -> Result<(), BoxError> {
    info!("checking for updates");

    let mut update_builder = configure_builder();
    update_builder
        .show_download_progress(true)
        .show_output(false)
        .no_confirm(no_confirm);

    if let Some(token) = auth_token {
        update_builder.auth_token(token);
    }

    if install(&update_builder)? {
        spawn_updated_version()?;
    } else {
        info!("already up-to-date");
    }

    Ok(())
}

/// Updates without asking or showing progress, for the GUI once the user agreed.
/// Returns true if the update was successful and the app should restart
#[cfg(feature = "gui")]
pub fn update_noninteractive() -> Result<bool, BoxError> {
    let mut update_builder = configure_builder();
    update_builder.show_download_progress(false).show_output(false).no_confirm(true);
    install(&update_builder)
}

/// Checks for a newer release without prompting or downloading, and returns its version
#[cfg(feature = "gui")]
pub fn check_for_update() -> Result<Option<String>, BoxError> {
    use self_update::backends::github::ReleaseList;

    let releases = ReleaseList::configure()
        .repo_owner(REPO_OWNER)
        .repo_name(REPO_NAME)
        .build()?
        .fetch()?;

    let identifier = identifier();
    let current_version = cargo_crate_version!();

    let latest_release = releases.iter().find(|r| r.asset_for(TARGET, identifier).is_some());

    let latest_release = match latest_release {
        Some(r) => r,
        None => return Ok(None),
    };

    let latest_version = &latest_release.version;
    if !self_update::version::bump_is_greater(current_version, latest_version)? {
        return Ok(None);
    }

    Ok(Some(latest_version.clone()))
}

/// Spawn the new version after update and exit
pub fn spawn_updated_version() -> Result<(), BoxError> {
    let current_exe = env::current_exe()?;
    let mut command = Command::new(current_exe);
    command.args(env::args().skip(1)).env("NO_SELF_UPDATE", "1");

    command.spawn()?;

    std::process::exit(0);
}
