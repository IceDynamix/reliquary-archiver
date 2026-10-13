#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]

use std::panic::AssertUnwindSafe;
use std::path::PathBuf;

use clap::Parser;
use futures::FutureExt;
use tracing::{debug, error, info};

#[cfg(feature = "stream")]
mod websocket;

#[cfg(feature = "gui")]
mod rgui;

#[cfg(windows)]
mod update;

#[cfg(all(not(feature = "pcap"), feature = "pktmon"))]
mod elevation;

#[cfg(feature = "gui")]
mod worker;

mod accounts;
mod capture;
mod cli;
mod logging;
#[cfg(feature = "gui")]
mod scopefns;
mod session;

// The GUI reads the log buffer to display it
#[cfg(feature = "gui")]
pub use logging::{LOG_BUFFER, LOG_NOTIFY, VEC_LAYER_HANDLE};

#[derive(Parser, Debug, Clone)]
struct Args {
    #[arg()]
    /// Path to output .json file to, per default: archive_output-%Y-%m-%dT%H-%M-%S.json
    output: Option<PathBuf>,

    /// Read packets from .pcap file instead of capturing live packets
    #[cfg(feature = "pcap")]
    #[arg(long)]
    pcap: Option<PathBuf>,

    /// Read packets from .etl file instead of capturing live packets
    #[cfg(feature = "pktmon")]
    #[arg(long)]
    etl: Option<PathBuf>,

    /// How long to wait in seconds until timeout is triggered for live captures
    #[arg(long, default_value_t = 120)]
    timeout: u64,

    /// Host a websocket server to stream relic/lc updates in real-time.
    /// This also disables the timeout
    #[cfg(feature = "stream")]
    #[arg(short, long)]
    stream: bool,

    /// Port to listen on for the websocket server, defaults to 23313
    #[cfg(feature = "stream")]
    #[arg(short = 'p', long, default_value_t = 23313)]
    websocket_port: u16,

    /// How verbose the output should be, can be set up to 3 times. Has no effect if RUST_LOG is set
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Path to output log to
    #[arg(short, long)]
    log_path: Option<PathBuf>,

    /// Don't check for updates, only applicable on Windows
    #[arg(long)]
    no_update: bool,

    /// Update without asking for confirmation, only applicable on Windows
    #[arg(long)]
    always_update: bool,

    /// Github Auth token to use when checking for updates, only applicable on Windows
    #[arg(long)]
    auth_token: Option<String>,

    /// Don't wait for enter to be pressed after capturing
    #[arg(short, long)]
    exit_after_capture: bool,
    /// Run in headless mode (no GUI), only applicable when GUI feature is enabled
    #[cfg(feature = "gui")]
    #[arg(long, short = 'H', visible_alias = "cli", visible_alias = "nogui")]
    headless: bool,

    /// Detach from the parent terminal (run in background), only applicable on Windows
    #[cfg(all(windows, feature = "gui"))]
    #[arg(long, short = 'd')]
    detach: bool,
}

#[cfg(not(any(feature = "pktmon", feature = "pcap")))]
compile_error!("Either \"pktmon\" (windows exclusive) or \"pcap\" must be enabled");

#[cfg(all(not(windows), feature = "gui"))]
compile_error!("GUI is only available on windows");

#[cfg(all(not(windows), feature = "pktmon"))]
compile_error!("The \"pktmon\" capture backend is only available on windows");

#[tokio::main]
async fn main() {
    color_eyre::install().unwrap();

    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        old_hook(panic_info);
        error!("Backtrace: {:#?}", backtrace);
    }));

    // Attach to parent console on Windows GUI builds
    // This is needed for --help output and headless mode to be visible
    // AttachConsole fails if no parent console exists (e.g. double-clicked from Explorer)
    #[cfg(all(windows, feature = "gui"))]
    let has_console =
        unsafe { windows::Win32::System::Console::AttachConsole(windows::Win32::System::Console::ATTACH_PARENT_PROCESS).is_ok() };

    let args = Args::parse();

    // Detach from parent terminal if requested
    #[cfg(all(windows, feature = "gui"))]
    if args.detach {
        unsafe { windows::Win32::System::Console::FreeConsole().ok() };
    }

    // Allocate a console for headless mode if AttachConsole didn't work
    #[cfg(all(windows, feature = "gui"))]
    if args.headless && !has_console && !args.detach {
        unsafe { windows::Win32::System::Console::AllocConsole().ok() };
    }

    logging::tracing_init(args.verbose, args.log_path.as_deref());

    debug!(?args);

    // AssertUnwindSafe is justified as all we do is write a crash log before ending the program and therefore there is no risk posed by potential broken invariants
    if let Err(payload) = AssertUnwindSafe(run(args)).catch_unwind().await {
        error!("the application panicked, this is a bug, please report it on GitHub or Discord");

        logging::write_crashlog(payload.as_ref());

        info!("press enter to close");
        std::io::stdin().read_line(&mut String::new()).unwrap();
    }
}

async fn run(args: Args) {
    // Only self update on Windows, since that's the only platform we ship releases for
    // In GUI mode, the update check is handled by the GUI after it launches
    #[cfg(windows)]
    {
        #[cfg(feature = "gui")]
        let gui_mode = !args.headless;
        #[cfg(not(feature = "gui"))]
        let gui_mode = false;

        if !gui_mode && !args.no_update && !std::env::var("NO_SELF_UPDATE").is_ok_and(|v| v == "1") {
            if let Err(e) = update::update_interactive(args.auth_token.as_deref(), args.always_update) {
                error!("Failed to update: {}", e);
            }
        }
    }

    #[cfg(all(not(feature = "pcap"), feature = "pktmon"))]
    if !elevation::is_admin() {
        elevation::prompt_and_escalate();
        return;
    }

    #[cfg(feature = "gui")]
    if !args.headless {
        rgui::run().unwrap();

        // Check if we need to spawn the updated version after GUI exits
        if update::should_spawn_after_exit() {
            if let Err(e) = update::spawn_updated_version() {
                error!("Failed to spawn updated version: {}", e);
            }
        }
        return;
    }

    cli::run(args).await;
}
