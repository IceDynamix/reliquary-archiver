use std::fs::File;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[cfg(feature = "pcap")] use capture::PCAP_FILTER;

use chrono::Local;
use clap::Parser;
use reliquary::network::command::command_id::{PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::{ConnectionPacket, GamePacket, GameSniffer};
use tracing::{debug, info, instrument, warn};
use tracing_subscriber::{prelude::*, EnvFilter, Layer, Registry};

#[cfg(windows)] use {
    std::env,
    std::process::Command,
    self_update::cargo_crate_version,
    tracing::error,
};

use reliquary_archiver::export::database::Database;
use reliquary_archiver::export::fribbels::OptimizerExporter;
use reliquary_archiver::export::Exporter;

mod capture;

#[derive(Parser, Debug)]
struct Args {
    #[arg()]
    /// Path to output .json file to, per default: archive_output-%Y-%m-%dT%H-%M-%S.json
    output: Option<PathBuf>,
    /// Read packets from .pcap file instead of capturing live packets
    #[cfg(feature = "pcap")]
    #[arg(long)]
    pcap: Option<PathBuf>,
    /// How long to wait in seconds until timeout is triggered for live captures
    #[arg(long, default_value_t = 120)]
    timeout: u64,
    /// How verbose the output should be, can be set up to 3 times. Has no effect if RUST_LOG is set
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
    /// Path to output log to
    #[arg(short, long)]
    log_path: Option<PathBuf>,
    /// Don't check for updates, only applicable on Windows
    #[arg(long)]
    no_update: bool,
    /// Github Auth token to use when checking for updates, only applicable on Windows
    #[arg(long)]
    auth_token: Option<String>,
    /// Don't wait for enter to be pressed after capturing
    #[arg(short, long)]
    exit_after_capture: bool,
}

fn main() {
    color_eyre::install().unwrap();
    let args = Args::parse();

    tracing_init(&args);

    debug!(?args);

    // Only self update on Windows, since that's the only platform we ship releases for
    #[cfg(windows)] {
        if !args.no_update && !env::var("NO_SELF_UPDATE").map_or(false, |v| v == "1") {
            if let Err(e) = update(args.auth_token.as_deref()) {
                error!("Failed to update: {}", e);
            }
        }
    }

    let database = Database::new();
    let sniffer = GameSniffer::new().set_initial_keys(database.keys.clone());
    let exporter = OptimizerExporter::new(database);

    #[cfg(feature = "pcap")]
    let export = match args.pcap {
        Some(_) => file_capture(&args, exporter, sniffer),
        None => live_capture(&args, exporter, sniffer),
    };

    #[cfg(not(feature = "pcap"))]
    let export = live_capture(&args, exporter, sniffer);

    if let Some(export) = export {
        let output_file = match args.output {
            Some(out) => out,
            _ => PathBuf::from(Local::now().format("archive_output-%Y-%m-%dT%H-%M-%S.json").to_string()),
        };
        let file = File::create(&output_file).unwrap();
        serde_json::to_writer_pretty(&file, &export).unwrap();
        info!(
            "wrote output to {}",
            output_file.canonicalize().unwrap().display()
        );
    } else {
        warn!("skipped writing output");
    }

    if let Some(log_path) = args.log_path {
        info!("wrote logs to {}", log_path.display());
    }

    if !args.exit_after_capture {
        info!("press enter to close");
        std::io::stdin().read_line(&mut String::new()).unwrap();
    }
}

#[cfg(windows)]
fn update(auth_token: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    info!("checking for updates");

    let mut update_builder = self_update::backends::github::Update::configure();

    update_builder
        .repo_owner("IceDynamix")
        .repo_name("reliquary-archiver")
        .bin_name("reliquary-archiver")
        .target("x64")
        .show_download_progress(true)
        .show_output(false)
        .current_version(cargo_crate_version!());

    if let Some(token) = auth_token {
        update_builder.auth_token(token);
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

fn tracing_init(args: &Args) {
    let env_filter = EnvFilter::builder()
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
        .from_env_lossy();

    let stdout_log = tracing_subscriber::fmt::layer()
        .with_ansi(false)
        .with_filter(env_filter);

    let subscriber = Registry::default().with(stdout_log);

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

    let subscriber = subscriber.with(file_log);

    tracing::subscriber::set_global_default(subscriber).expect("unable to set up logging");
}

#[cfg(feature = "pcap")]
#[instrument(skip_all)]
fn file_capture<E>(args: &Args, mut exporter: E, mut sniffer: GameSniffer) -> Option<E::Export>
where
    E: Exporter,
{
    let mut capture =
        pcap::Capture::from_file(args.pcap.as_ref().unwrap()).expect("could not read pcap file");

    capture.filter(PCAP_FILTER, false).unwrap();

    info!("capturing");
    while let Ok(packet) = capture.next_packet() {
        info!("received packet: {:?}", packet.data.to_vec());
        if let Ok(packets) = sniffer.receive_packet(packet.data.to_vec()) {
            for packet in packets {
                match packet {
                    GamePacket::Commands(command) => {
                        match command {
                            Ok(command) => {
                                exporter.read_command(command);
                            }
                            Err(e) => {
                                warn!(%e);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    exporter.export()
}

#[instrument(skip_all)]
fn live_capture<E>(args: &Args, mut exporter: E, mut sniffer: GameSniffer) -> Option<E::Export>
where
    E: Exporter,
{
    let abort_signal = Arc::new(AtomicBool::new(false));

    {
        let abort_signal = abort_signal.clone();
        if let Err(e) = ctrlc::set_handler(move || {
            abort_signal.store(true, Ordering::Relaxed);
        }) {
            error!("Failed to set Ctrl-C handler: {}", e);
        }
    }

    #[cfg(windows)]
    let rx = {
        #[cfg(feature = "pcap")] {
            capture::listen_on_all::<capture::pcap::PcapBackend>(abort_signal.clone())
        }

        #[cfg(all(not(feature = "pcap"), feature = "pktmon"))] {
            if unsafe { windows::Win32::UI::Shell::IsUserAnAdmin().into() } {
                capture::listen_on_all::<capture::pktmon::PktmonBackend>(abort_signal.clone())
            } else {
                warn!("You must run this program as an administrator to capture packets");
                return None;
            }
        }
    };

    #[cfg(not(windows))]
    let rx = capture::listen_on_all::<capture::pcap::PcapBackend>(abort_signal.clone());

    let (rx, join_handles) = rx.expect("Failed to start packet capture");

    info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");
    info!("listening with a timeout of {} seconds...", args.timeout);

    'recv: loop {
        match rx.recv_timeout(Duration::from_secs(args.timeout)) {
            Ok(packet) => {
                match sniffer.receive_packet(packet.data) {
                    Ok(packets) => {
                        for packet in packets {
                            match packet {
                                GamePacket::Connection(c) => {
                                    match c {
                                        ConnectionPacket::HandshakeEstablished => {
                                            info!("detected connection established");
                                        }
                                        ConnectionPacket::Disconnected => {
                                            // program is probably going to exit before this happens
                                            // info!("detected connection disconnected");
                                        }
                                        _ => {}
                                    }
                                }
                                GamePacket::Commands(command) => {
                                    match command {
                                        Ok(command) => {
                                            if command.command_id == PlayerLoginScRsp {
                                                info!("detected login");
                                            }
            
                                            if command.command_id == PlayerLoginFinishScRsp {
                                                info!("detected login end, assume initialization is finished");
                                                break 'recv;
                                            }
            
                                            exporter.read_command(command);
                                        }
                                        Err(e) => {
                                            warn!(%e);
                                            break;
                                        }
                                    }
                                }
                            }
                        }

                        if exporter.is_finished() {
                            info!("retrieved all relevant packets, stop listening");
                            break 'recv;
                        }
                    }
                    Err(e) => {
                        warn!(%e);
                        break;
                    }
                }
            }
            Err(e) => {
                warn!(%e);
                break;
            }
        }
    }

    abort_signal.store(true, Ordering::Relaxed);
    for handle in join_handles {
        handle.join().expect("Failed to join capture thread");
    }

    exporter.export()
}
