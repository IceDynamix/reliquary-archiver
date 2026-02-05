#![cfg_attr(all(windows, feature = "gui"), windows_subsystem = "windows")]
#![allow(unused)]

use std::collections::HashSet;
use std::fs::File;
use std::io;
use std::io::Write;
use std::panic::AssertUnwindSafe;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, LazyLock, LockResult, Mutex, TryLockResult};
use std::time::Duration;

#[cfg(feature = "pcap")]
use capture::PCAP_FILTER;
use chrono::Local;
use clap::Parser;
use futures::lock::Mutex as FuturesMutex;
use futures::{future, select, FutureExt, StreamExt};
use reliquary::network::command::command_id::{PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::command::GameCommandError;
use reliquary::network::{ConnectionPacket, GamePacket, GameSniffer, NetworkError};
use tokio::pin;
use tracing::instrument::WithSubscriber;
use tracing::level_filters::LevelFilter;
use tracing::{debug, error, info, instrument, warn};
use tracing_subscriber::filter::Filtered;
use tracing_subscriber::fmt::{MakeWriter, SubscriberBuilder};
use tracing_subscriber::prelude::*;
use tracing_subscriber::{reload, EnvFilter, Layer, Registry};

#[cfg(feature = "stream")]
mod websocket;

#[cfg(feature = "gui")]
mod rgui;

#[cfg(windows)]
mod update;

use reliquary_archiver::export::database::Database;
use reliquary_archiver::export::fribbels::OptimizerExporter;
use reliquary_archiver::export::Exporter;

mod capture;
mod scopefns;
mod worker;

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

#[derive(Debug, Clone)]
enum CaptureMode {
    Live,
    #[cfg(feature = "pcap")]
    Pcap(PathBuf),
    #[cfg(feature = "pktmon")]
    Etl(PathBuf),
}

impl CaptureMode {
    fn from_args(args: &Args) -> Self {
        #[cfg(feature = "pcap")]
        if let Some(path) = &args.pcap {
            return CaptureMode::Pcap(path.clone());
        }

        #[cfg(feature = "pktmon")]
        if let Some(path) = &args.etl {
            return CaptureMode::Etl(path.clone());
        }

        CaptureMode::Live
    }
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

    // Copy the exit_after_capture flag to a local variable before args is moved into the closure
    let exit_after_capture = args.exit_after_capture;

    tracing_init(&args);

    debug!(?args);

    // AssertUnwindSafe is justified as all we do is write a crash log before ending the program and therefore there is no risk posed by potential broken invariants
    if let Err(payload) = AssertUnwindSafe(capture(args)).catch_unwind().await {
        error!("the application panicked, this is a bug, please report it on GitHub or Discord");

        // Write crashlog
        if let Ok(mut file) = File::create("crashlog.txt") {
            if let TryLockResult::Ok(buffer) = LOG_BUFFER.try_lock() {
                let lines = buffer.join("\n");
                file.write_all(lines.as_bytes()).unwrap();
            } else {
                file.write_all("failed to lock log buffer".as_bytes()).unwrap();
            }
            file.write_all("\n\n".as_bytes()).unwrap();
            if let Some(s) = payload.downcast_ref::<&str>() {
                file.write_all(s.as_bytes()).unwrap();
            } else if let Some(s) = payload.downcast_ref::<String>() {
                file.write_all(s.as_bytes()).unwrap();
            } else {
                file.write_all("panic: unknown payload type".as_bytes()).unwrap();
            }
            info!("wrote crashlog to crashlog.txt");
        }

        info!("press enter to close");
        std::io::stdin().read_line(&mut String::new()).unwrap();
    }
}

async fn capture(args: Args) {
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
    if !unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().into() } {
        fn confirm(msg: &str) -> bool {
            use std::io::Write;

            print!("{}", msg);
            if std::io::stdout().flush().is_err() {
                return false;
            }

            let mut input = String::new();
            if std::io::stdin().read_line(&mut input).is_err() {
                return false;
            }

            input = input.trim().to_lowercase();
            input.starts_with("y") || input.is_empty()
        }

        println!();
        println!("===========================================================================================================");
        println!("                       Administrative privileges are required to capture packets");
        println!("===========================================================================================================");
        println!();
        println!("Reliquary Archiver now uses PacketMonitor (pktmon) to capture the game traffic instead of npcap on Windows");
        println!("Due to the way pktmon works, it requires running the application as an administrator");
        println!();
        println!("If you don't feel comfortable running the application as an administrator, you can download the pcap");
        println!("version of Reliquary Archiver from the GitHub releases page.");
        println!();
        if confirm("Would you like to restart the application with elevated privileges? (Y/n): ") {
            if let Err(e) = escalate_to_admin() {
                error!("Failed to escalate privileges: {}", e);
            }
        }

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

    // Headless/CLI mode
    {
        let database = Database::new();
        let sniffer = GameSniffer::new().set_initial_keys(database.keys.clone());
        let exporter = OptimizerExporter::new();

        let capture_mode = CaptureMode::from_args(&args);
        let export = match capture_mode {
            CaptureMode::Live => live_capture_wrapper(&args, exporter, sniffer).await,
            #[cfg(feature = "pcap")]
            CaptureMode::Pcap(path) => capture_from_pcap(exporter, sniffer, path),
            #[cfg(feature = "pktmon")]
            CaptureMode::Etl(path) => capture_from_etl(exporter, sniffer, path),
        };

        if let Some(export) = export {
            let file_name = Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string();
            let mut output_file = match args.output {
                Some(out) => out,
                _ => PathBuf::from(file_name.clone()),
            };

            macro_rules! pick_file {
                () => {
                    if let Some(new_path) = rfd::FileDialog::new()
                        .set_title("Select output file location")
                        .set_file_name(&file_name)
                        .add_filter("JSON files", &["json"])
                        .save_file()
                    {
                        output_file = new_path;
                        continue;
                    } else {
                        error!("No alternative path selected, aborting write");
                        break;
                    }
                };
            }
            info!("exporting collected data");
            loop {
                match File::create(&output_file) {
                    Ok(file) => {
                        if let Err(e) = serde_json::to_writer_pretty(&file, &export) {
                            error!("Failed to write to {}: {}", output_file.display(), e);
                            pick_file!();
                        }
                        info!("wrote output to {}", output_file.canonicalize().unwrap().display());
                        break;
                    }
                    Err(e) => {
                        error!("Failed to create file at {}: {}", output_file.display(), e);
                        pick_file!();
                    }
                }
            }
        } else {
            warn!("skipped writing output");
        }
        if let Some(log_path) = args.log_path {
            info!("wrote logs to {}", log_path.display());
        }
    }
}

struct VecWriter;

impl VecWriter {
    pub fn new() -> Self {
        Self
    }
}

pub static LOG_BUFFER: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));
pub static LOG_NOTIFY: LazyLock<tokio::sync::Notify> = LazyLock::new(tokio::sync::Notify::new);

type VecLayerHandle = Box<dyn Fn(LevelFilter) + Send>;
pub static VEC_LAYER_HANDLE: LazyLock<Mutex<Option<VecLayerHandle>>> = LazyLock::new(|| Mutex::new(None));

impl std::io::Write for VecWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let str = String::from_utf8_lossy(buf);
        let lines = str.lines().map(|s| s.to_string());
        LOG_BUFFER.lock().unwrap().extend(lines);
        LOG_NOTIFY.notify_one();
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct DualWriter<A: io::Write, B: io::Write> {
    m: Arc<Mutex<(A, B)>>,
}

impl<A: io::Write, B: io::Write> DualWriter<A, B> {
    fn new(a: A, b: B) -> Self {
        Self {
            m: Arc::new(Mutex::new((a, b))),
        }
    }
}

impl<A: io::Write, B: io::Write> io::Write for DualWriter<A, B> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut m = self.m.lock().unwrap();
        m.0.write(buf)?;
        m.1.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut m = self.m.lock().unwrap();
        m.0.flush()?;
        m.1.flush()
    }
}

impl<'a, A: io::Write, B: io::Write> MakeWriter<'a> for DualWriter<A, B> {
    type Writer = DualWriter<A, B>;

    fn make_writer(&'a self) -> Self::Writer {
        DualWriter { m: self.m.clone() }
    }
}

fn tracing_init(args: &Args) {
    tracing_log::LogTracer::init().unwrap();

    fn env_filter(args: &Args) -> EnvFilter {
        EnvFilter::builder()
            .with_default_directive(
                match args.verbose {
                    0 => "reliquary_archiver=info",
                    1 => "info",
                    2 => "debug",
                    _ => "trace",
                }
                .parse()
                .unwrap(),
            )
            .from_env_lossy()
    }

    let subscriber = tracing_subscriber::fmt::fmt()
        .with_ansi(false)
        .with_env_filter(env_filter(args))
        .with_writer(DualWriter::new(VecWriter::new(), io::stdout()))
        .with_filter_reloading();

    let handle = subscriber.reload_handle();
    *VEC_LAYER_HANDLE.lock().unwrap() = Some(Box::new(move |l| {
        handle.modify(|f| {
            *f = EnvFilter::builder()
                .parse(match l {
                    LevelFilter::TRACE => "trace",
                    LevelFilter::DEBUG => "debug",
                    LevelFilter::INFO => "info",
                    LevelFilter::WARN => "warn",
                    LevelFilter::ERROR => "error",
                    _ => "off",
                })
                .unwrap();
        });
    }));

    let file_log = if let Some(log_path) = &args.log_path {
        let log_file = File::create(log_path).unwrap();
        let file_log = tracing_subscriber::fmt::layer()
            .json()
            .with_writer(Mutex::new(log_file))
            .with_filter(tracing::level_filters::LevelFilter::TRACE);
        Some(file_log)
    } else {
        None
    };

    let subscriber = subscriber.finish();

    let subscriber = subscriber.with(file_log);

    tracing::subscriber::set_global_default(subscriber).expect("unable to set up logging");
}

// Helper function to process a packet and determine if capture should stop
enum ProcessResult {
    Continue,
    Stop,
}

// Simplified packet processing for file captures
fn file_process_packet<E>(exporter: &mut E, sniffer: &mut GameSniffer, payload: Vec<u8>) -> ProcessResult
where
    E: Exporter,
{
    if let Ok(packets) = sniffer.receive_packet(payload) {
        for packet in packets {
            if let GamePacket::Commands { conv_id, result } = packet {
                match result {
                    Ok(command) => {
                        exporter.read_command(command);

                        if exporter.is_initialized() {
                            info!("finished capturing");
                            return ProcessResult::Stop;
                        }
                    }
                    Err(e) => {
                        warn!(conv_id, %e);
                        if matches!(e, GameCommandError::VersionMismatch) {
                            // Client packet was misordered from server packet
                            // This will be reprocessed after we receive the new session key
                            return ProcessResult::Continue;
                        }

                        return ProcessResult::Stop;
                    }
                }
            }
        }
    }

    ProcessResult::Continue
}

#[instrument(skip_all)]
#[cfg(feature = "pcap")]
pub fn capture_from_pcap<E>(mut exporter: E, mut sniffer: GameSniffer, pcap_path: PathBuf) -> Option<E::Export>
where
    E: Exporter,
{
    info!("Capturing from pcap file: {}", pcap_path.display());
    let mut capture = pcap::Capture::from_file(&pcap_path).expect("could not read pcap file");
    capture.filter(PCAP_FILTER, false).unwrap();

    while let Ok(packet) = capture.next_packet() {
        match file_process_packet(&mut exporter, &mut sniffer, packet.data.to_vec()) {
            ProcessResult::Continue => {}
            ProcessResult::Stop => break,
        }
    }

    exporter.export()
}

#[instrument(skip_all)]
#[cfg(feature = "pktmon")]
fn capture_from_etl<E>(mut exporter: E, mut sniffer: GameSniffer, etl_path: PathBuf) -> Option<E::Export>
where
    E: Exporter,
{
    info!("Capturing from etl file: {}", etl_path.display());
    let mut capture = pktmon::EtlCapture::new(&etl_path).expect("could not read etl file");
    capture.start().expect("could not start etl capture");

    while let Ok(packet) = capture.next_packet() {
        match packet.payload {
            pktmon::PacketPayload::Ethernet(payload) => match file_process_packet(&mut exporter, &mut sniffer, payload) {
                ProcessResult::Continue => {}
                ProcessResult::Stop => break,
            },
            _ => {}
        }
    }

    capture.stop().expect("could not stop etl capture");
    exporter.export()
}

async fn live_capture_wrapper<E>(args: &Args, exporter: E, sniffer: GameSniffer) -> Option<E::Export>
where
    E: Exporter,
    E::Export: From<<OptimizerExporter as Exporter>::Export>,
{
    use reliquary_archiver::export::database::get_database;
    #[cfg(feature = "stream")]
    use tokio::sync::watch;

    #[cfg(feature = "stream")]
    use crate::websocket::{start_websocket_server, PortSource};
    use crate::worker::MultiAccountManager;

    #[cfg(not(feature = "stream"))]
    let streaming = false;

    #[cfg(feature = "stream")]
    let streaming = args.stream;

    // Always use MultiAccountManager for consistency
    let database = get_database();
    let manager = Arc::new(FuturesMutex::new(MultiAccountManager::new(database.keys.clone())));

    #[cfg(feature = "stream")]
    let selected_account_tx = if streaming {
        let (tx, rx) = watch::channel::<Option<u32>>(None);

        // Start websocket server
        tokio::spawn(start_websocket_server(PortSource::Fixed(args.websocket_port), manager.clone(), rx));

        info!("WebSocket server starting on port {}...", args.websocket_port);
        Some(tx)
    } else {
        None
    };

    #[cfg(not(feature = "stream"))]
    let selected_account_tx = None;

    // Run live capture with manager
    let result = live_capture(args, manager, sniffer, selected_account_tx, streaming).await;
    result.map(|export| export.into())
}

#[instrument(skip_all)]
async fn live_capture(
    args: &Args,
    manager: Arc<FuturesMutex<worker::MultiAccountManager>>,
    mut sniffer: GameSniffer,
    selected_account_tx: Option<tokio::sync::watch::Sender<Option<u32>>>,
    streaming: bool,
) -> Option<<OptimizerExporter as Exporter>::Export> {
    use reliquary::network::command::command_id::{PlayerGetTokenScRsp, PlayerLoginFinishScRsp, PlayerLoginScRsp};
    use reliquary::network::command::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp as PlayerGetTokenScRspProto;

    let rx = {
        #[cfg(feature = "pcap")]
        {
            capture::listen_on_all(capture::pcap::PcapBackend)
        }

        #[cfg(all(not(feature = "pcap"), feature = "pktmon"))]
        {
            capture::listen_on_all(capture::pktmon::PktmonBackend)
        }
    };

    let packet_stream = rx.expect("Failed to start packet capture");
    let mut packet_stream = packet_stream.fuse();

    info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");

    if streaming {
        info!("WebSocket streaming enabled - capture will run until manually stopped");
    }

    pin!(
        let timeout_future = maybe_timeout(
            if !streaming {
                info!("listening with a timeout of {} seconds...", args.timeout);
                Some(Duration::from_secs(args.timeout))
            } else {
                None
            }
        ).fuse();
    );

    let mut poisoned_sources = HashSet::new();
    let mut latest_uid: Option<u32> = None;

    'recv: loop {
        let received = select! {
            packet = packet_stream.next() => match packet {
                Some(packet) => packet,
                None => break 'recv,
            },

            _ = timeout_future => {
                break 'recv;
            }
        };

        match received {
            Ok(packet) => {
                if poisoned_sources.contains(&packet.source_id) {
                    // We already know that this source is poisoned, so we can skip it
                    continue;
                }

                match sniffer.receive_packet(packet.data) {
                    Ok(packets) => {
                        for packet in packets {
                            match packet {
                                GamePacket::Connection(c) => match c {
                                    ConnectionPacket::HandshakeEstablished { conv_id } => {
                                        info!(conv_id, "detected connection established");

                                        if cfg!(all(feature = "pcap", windows)) {
                                            info!("If the program gets stuck at this point for longer than 10 seconds, please try the pktmon release from https://github.com/IceDynamix/reliquary-archiver/releases/latest");
                                        }
                                    }
                                    ConnectionPacket::Disconnected => {
                                        info!("detected connection disconnected");
                                    }
                                    _ => {}
                                },
                                GamePacket::Commands { conv_id, result } => match result {
                                    Ok(command) => {
                                        if command.command_id == PlayerLoginScRsp {
                                            info!(conv_id, "detected login start");
                                        }

                                        // Check for UID discovery to register with manager
                                        if command.command_id == PlayerGetTokenScRsp {
                                            if let Ok(token_rsp) = command.parse_proto::<PlayerGetTokenScRspProto>() {
                                                let uid = token_rsp.uid;
                                                let mut mgr = manager.lock().await;
                                                mgr.register_uid(conv_id, uid);

                                                // Auto-select latest account
                                                latest_uid = Some(uid);
                                                if let Some(ref tx) = selected_account_tx {
                                                    tx.send(Some(uid)).ok();
                                                    info!(uid, "Auto-selected account for WebSocket streaming");
                                                }
                                            }
                                        }

                                        if !streaming && command.command_id == PlayerLoginFinishScRsp {
                                            info!("detected login end, assume initialization is finished");
                                            break 'recv;
                                        }

                                        // Route command to correct account exporter
                                        let exporter = {
                                            let mut mgr = manager.lock().await;
                                            mgr.get_or_create_exporter(conv_id)
                                        };
                                        exporter.lock().await.read_command(command);
                                    }
                                    Err(e) => {
                                        warn!(conv_id, %e);
                                        if matches!(e, GameCommandError::VersionMismatch) {
                                            // Client packet was misordered from server packet
                                            // This will be reprocessed after we receive the new session key
                                        } else {
                                            break 'recv;
                                        }
                                    }
                                },
                            }
                        }

                        // Check if initialized for early exit in non-streaming mode
                        if !streaming {
                            if let Some(uid) = latest_uid {
                                let mgr = manager.lock().await;
                                if let Some(exporter) = mgr.get_account_exporter(uid) {
                                    if exporter.lock().await.is_initialized() {
                                        info!("retrieved all relevant packets, stop listening");
                                        break 'recv;
                                    }
                                }
                            }
                        }
                    }
                    Err(e) => {
                        warn!(%e);
                        match e {
                            NetworkError::ConnectionPacket(_) => {
                                // Connection errors are not fatal as all network interfaces are funneled through the same stream
                                // Just mark this source as poisoned and continue listening on other sources
                                poisoned_sources.insert(packet.source_id);
                                continue;
                            }
                            _ => break 'recv,
                        }
                    }
                }
            }
            Err(e) => {
                warn!(%e);
                break 'recv;
            }
        }
    }

    // Export from the latest account
    if let Some(uid) = latest_uid {
        let mgr = manager.lock().await;
        if let Some(exporter) = mgr.get_account_exporter(uid) {
            return exporter.lock().await.export();
        }
    }

    None
}

async fn maybe_timeout(timeout: Option<Duration>) -> () {
    if let Some(timeout) = timeout {
        tokio::time::sleep(timeout).await;
    } else {
        future::pending::<()>().await;
    }
}

#[cfg(all(not(feature = "pcap"), feature = "pktmon"))]
fn escalate_to_admin() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;

    use windows::core::{w, PCWSTR};
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NO_CONSOLE, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_OWNER, SW_SHOWNORMAL};

    let args_str = std::env::args().skip(1).collect::<Vec<_>>().join(" ");

    let exe_path = std::env::current_exe()
        .expect("Failed to get current exe")
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let args = args_str.encode_utf16().chain(Some(0)).collect::<Vec<_>>();

    unsafe {
        let mut options = SHELLEXECUTEINFOW {
            cbSize: size_of::<SHELLEXECUTEINFOW>() as u32,
            fMask: SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NO_CONSOLE,
            hwnd: GetWindow(GetConsoleWindow(), GW_OWNER).unwrap_or(GetConsoleWindow()),
            lpVerb: w!("runas"),
            lpFile: PCWSTR(exe_path.as_ptr()),
            lpParameters: PCWSTR(args.as_ptr()),
            lpDirectory: PCWSTR::null(),
            nShow: SW_SHOWNORMAL.0,
            lpIDList: std::ptr::null_mut(),
            lpClass: PCWSTR::null(),
            dwHotKey: 0,
            ..Default::default()
        };

        ShellExecuteExW(&mut options)?;
    };

    // Exit the current process since we launched a new elevated one
    std::process::exit(0);
}
