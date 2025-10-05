// #![windows_subsystem = "windows"]
#![allow(unused)]

use std::collections::HashSet;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::RecvTimeoutError;
use std::sync::{Arc, LazyLock, LockResult, Mutex, TryLockResult};
use std::time::Duration;
use std::{io, panic};

#[cfg(feature = "pcap")]
use capture::PCAP_FILTER;

use chrono::Local;
use clap::Parser;
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
use tracing_subscriber::{prelude::*, reload, EnvFilter, Layer, Registry};

#[cfg(windows)]
use {self_update::cargo_crate_version, std::env, std::process::Command};

#[cfg(feature = "stream")]
mod websocket;

use reliquary_archiver::export::database::Database;
use reliquary_archiver::export::fribbels::OptimizerExporter;
use reliquary_archiver::export::Exporter;

mod capture;
// mod gui;
mod rgui;
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
    /// Port to listen on for the websocket server, defaults to 53313
    #[cfg(feature = "stream")]
    #[arg(short = 'p', long, default_value_t = 53313)] // Seele :)
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

fn main() {
    color_eyre::install().unwrap();

    let old_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |panic_info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        old_hook(panic_info);
        error!("Backtrace: {:#?}", backtrace);
    }));

    let args = Args::parse();

    // Copy the exit_after_capture flag to a local variable before args is moved into the closure
    let exit_after_capture = args.exit_after_capture;

    tracing_init(&args);

    debug!(?args);

    if let Err(payload) = panic::catch_unwind(move || {
        // Only self update on Windows, since that's the only platform we ship releases for
        #[cfg(windows)]
        {
            if !args.no_update && !env::var("NO_SELF_UPDATE").map_or(false, |v| v == "1") {
                if let Err(e) = update(args.auth_token.as_deref(), args.always_update) {
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

        // gui::run().unwrap();
        rgui::run().unwrap();

        // let database = Database::new();
        // let sniffer = GameSniffer::new().set_initial_keys(database.keys.clone());
        // let exporter = OptimizerExporter::new(database);

        // let capture_mode = CaptureMode::from_args(&args);
        // let export = match capture_mode {
        //     CaptureMode::Live => live_capture_wrapper(&args, exporter, sniffer),
        //     #[cfg(feature = "pcap")]
        //     CaptureMode::Pcap(path) => capture_from_pcap(exporter, sniffer, path),
        //     #[cfg(feature = "pktmon")]
        //     CaptureMode::Etl(path) => capture_from_etl(exporter, sniffer, path),
        // };

        // if let Some(export) = export {
        //     let file_name = Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string();
        //     let mut output_file = match args.output {
        //         Some(out) => out,
        //         _ => PathBuf::from(file_name.clone()),
        //     };

        //     macro_rules! pick_file {
        //         () => {
        //             if let Some(new_path) = rfd::FileDialog::new()
        //                 .set_title("Select output file location")
        //                 .set_file_name(&file_name)
        //                 .add_filter("JSON files", &["json"])
        //                 .save_file()
        //             {
        //                 output_file = new_path;
        //                 continue;
        //             } else {
        //                 error!("No alternative path selected, aborting write");
        //                 break;
        //             }
        //         };
        //     }
        //     info!("exporting collected data");
        //     loop {
        //         match File::create(&output_file) {
        //             Ok(file) => {
        //                 if let Err(e) = serde_json::to_writer_pretty(&file, &export) {
        //                     error!("Failed to write to {}: {}", output_file.display(), e);
        //                     pick_file!();
        //                 }
        //                 info!(
        //                     "wrote output to {}",
        //                     output_file.canonicalize().unwrap().display()
        //                 );
        //                 break;
        //             }
        //             Err(e) => {
        //                 error!("Failed to create file at {}: {}", output_file.display(), e);
        //                 pick_file!();
        //             }
        //         }
        //     }
        // } else {
        //     warn!("skipped writing output");
        // }

        if let Some(log_path) = args.log_path {
            info!("wrote logs to {}", log_path.display());
        }
    }) {
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

#[cfg(windows)]
fn update(auth_token: Option<&str>, no_confirm: bool) -> Result<(), Box<dyn std::error::Error>> {
    info!("checking for updates");

    let mut update_builder = self_update::backends::github::Update::configure();

    update_builder
        .repo_owner("IceDynamix")
        .repo_name("reliquary-archiver")
        .bin_name("reliquary-archiver")
        .target("x64")
        .show_download_progress(true)
        .show_output(false)
        .no_confirm(no_confirm)
        .current_version(cargo_crate_version!());

    if let Some(token) = auth_token {
        update_builder.auth_token(token);
    }

    if cfg!(feature = "pcap") {
        update_builder.identifier("pcap");
    } else if cfg!(feature = "pktmon") {
        update_builder.identifier("pktmon");
    }

    let status = update_builder.build()?.update()?;

    if status.updated() {
        info!("updated to {}", status.version());

        let current_exe = env::current_exe();
        let mut command = Command::new(current_exe?);
        command.args(env::args().skip(1)).env("NO_SELF_UPDATE", "1");

        command.spawn().and_then(|mut c| c.wait())?;

        // Stop running the old version
        std::process::exit(0);
    } else {
        info!("already up-to-date");
    }

    Ok(())
}

struct VecWriter;

impl VecWriter {
    pub fn new() -> Self {
        Self
    }
}

pub static LOG_BUFFER: LazyLock<Mutex<Vec<String>>> = LazyLock::new(|| Mutex::new(Vec::new()));
pub static LOG_NOTIFY: LazyLock<tokio::sync::Notify> = LazyLock::new(|| tokio::sync::Notify::new());

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
    //layer().with_ansi(false).with_filter(env_filter(args));

    // Create the vec_layer with a reloadable filter
    // let vec_filter = env_filter(args);
    // Filtered<Layer<Registry, DefaultFields, Format, impl Fn() -> VecWriter>, Layer<EnvFilter, Registry>, Registry>

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

    // let (vec_filter, vec_reload_handle) = reload::Layer::new(vec_filter);
    // let vec_layer = tracing_subscriber::fmt::layer()
    //     .with_ansi(false)
    //     .with_writer(VecWriter::new)
    //     .with_filter(vec_filter);

    // // Store the reload handle globally
    // *VEC_LAYER_HANDLE.lock().unwrap() = Some(vec_reload_handle);

    // let builder = SubscriberBuilder::default().with_writer(VecWriter::new).finish();

    // let subscriber = Registry::default().with(stdout_log);

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
    // let subscriber = subscriber.with(builder);

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
            match packet {
                GamePacket::Commands(command) => match command {
                    Ok(command) => {
                        exporter.read_command(command);

                        if exporter.is_initialized() {
                            info!("finished capturing");
                            return ProcessResult::Stop;
                        }
                    }
                    Err(e) => {
                        warn!(%e);
                        if matches!(e, GameCommandError::VersionMismatch { .. }) {
                            // Client packet was misordered from server packet
                            // This will be reprocessed after we receive the new session key
                            return ProcessResult::Continue;
                        }

                        return ProcessResult::Stop;
                    }
                },
                _ => {}
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
{
    let exporter = Arc::new(Mutex::new(exporter));

    #[cfg(feature = "stream")]
    {
        if args.stream {
            let port = args.websocket_port;
            // let ws_server = rt.block_on(websocket::start_websocket_server(port, exporter.clone()));

            info!("WebSocket server running on ws://localhost:{}/ws", port);
            info!("You can connect to this WebSocket server to receive real-time relic updates");

            let result = live_capture(args, exporter, sniffer);

            // ws_server.abort();

            result.await
        } else {
            live_capture(args, exporter, sniffer).await
        }
    }

    #[cfg(not(feature = "stream"))]
    {
        live_capture(args, exporter, sniffer).await
    }
}

async fn maybe_timeout(timeout: Option<Duration>) -> () {
    if let Some(timeout) = timeout {
        tokio::time::sleep(timeout).await;
    } else {
        future::pending::<()>().await;
    }
}

#[instrument(skip_all)]
async fn live_capture<E>(args: &Args, exporter: Arc<Mutex<E>>, mut sniffer: GameSniffer) -> Option<E::Export>
where
    E: Exporter,
{
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

    #[cfg(feature = "stream")]
    let streaming = args.stream;

    #[cfg(not(feature = "stream"))]
    let streaming = false;

    info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");

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

    'recv: loop {
        // let received: Result<capture::Packet, CaptureError> = if streaming {
        //     // If streaming, we don't want to timeout during inactivity
        // //     rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        //     todo!()
        // } else {
        // //     rx.recv_timeout(Duration::from_secs(args.timeout))
        //     packet_stream.selec
        // };

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
                                    ConnectionPacket::HandshakeEstablished => {
                                        info!("detected connection established");

                                        if cfg!(all(feature = "pcap", windows)) {
                                            info!("If the program gets stuck at this point for longer than 10 seconds, please try the pktmon release from https://github.com/IceDynamix/reliquary-archiver/releases/latest");
                                        }
                                    }
                                    ConnectionPacket::Disconnected => {
                                        info!("detected connection disconnected");
                                    }
                                    _ => {}
                                },
                                GamePacket::Commands(command) => match command {
                                    Ok(command) => {
                                        if command.command_id == PlayerLoginScRsp {
                                            info!("detected login start");
                                        }

                                        if !streaming && command.command_id == PlayerLoginFinishScRsp {
                                            info!("detected login end, assume initialization is finished");
                                            break 'recv;
                                        }

                                        exporter.lock().unwrap().read_command(command);
                                    }
                                    Err(e) => {
                                        warn!(%e);
                                        if matches!(e, GameCommandError::VersionMismatch { .. }) {
                                            // Client packet was misordered from server packet
                                            // This will be reprocessed after we receive the new session key
                                        } else {
                                            break 'recv;
                                        }
                                    }
                                },
                            }
                        }

                        if !streaming && exporter.lock().unwrap().is_initialized() {
                            info!("retrieved all relevant packets, stop listening");
                            break 'recv;
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

    // abort_signal.store(true, Ordering::Relaxed);

    // #[cfg(target_os = "linux")] {
    //     // Detach join handles on linux since pcap timeout will not fire if no packets are received on some interface
    //     drop(join_handles);
    // }

    // TODO: determine why pcap timeout is not working on linux, so that we can gracefully exit
    // #[cfg(not(target_os = "linux"))] {
    //     for handle in join_handles {
    //         handle.join().expect("Failed to join capture thread");
    //     }
    // }

    // for handle in join_handles {
    //     handle.abort();
    // }

    exporter.lock().unwrap().export()
}

#[cfg(all(not(feature = "pcap"), feature = "pktmon"))]
fn escalate_to_admin() -> Result<(), Box<dyn std::error::Error>> {
    use std::os::windows::ffi::OsStrExt;
    use windows::core::w;
    use windows::core::PCWSTR;
    use windows::Win32::System::Console::GetConsoleWindow;
    use windows::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SEE_MASK_NO_CONSOLE, SHELLEXECUTEINFOW};
    use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_OWNER, SW_SHOWNORMAL};

    let args_str = env::args().skip(1).collect::<Vec<_>>().join(" ");

    let exe_path = env::current_exe()
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
