use std::fs::File;
use std::path::PathBuf;
use std::sync::{mpsc, Mutex};
use std::time::Duration;

use chrono::Local;
use clap::Parser;
use pcap::{ConnectionStatus, Device, Error};
use reliquary::network::gen::command_id::{PlayerLoginFinishScRsp, PlayerLoginScRsp};
use reliquary::network::{ConnectionPacket, GamePacket, GameSniffer};
use tracing::{debug, info, instrument, trace, warn};
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

const PACKET_FILTER: &str = "udp portrange 23301-23302";

#[derive(Parser, Debug)]
struct Args {
    #[arg()]
    /// Path to output .json file to, per default: archive_output-%Y-%m-%dT%H-%M-%S.json
    output: Option<PathBuf>,
    /// Read packets from .pcap file instead of capturing live packets
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

    let export = match args.pcap {
        Some(_) => file_capture(&args, exporter, sniffer),
        None => live_capture(&args, exporter, sniffer),
    };

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

#[instrument(skip_all)]
fn file_capture<E>(args: &Args, mut exporter: E, mut sniffer: GameSniffer) -> Option<E::Export>
where
    E: Exporter,
{
    let mut capture =
        pcap::Capture::from_file(args.pcap.as_ref().unwrap()).expect("could not read pcap file");

    capture.filter(PACKET_FILTER, false).unwrap();

    let mut invalid = 0;

    info!("capturing");
    while let Ok(packet) = capture.next_packet() {
        if let Some(GamePacket::Commands(commands)) = sniffer.receive_packet(packet.data.to_vec()) {
            if commands.is_empty() {
                invalid += 1;

                // FIXME: disable the invalid packet checks until the situation in
                // reliquary lib has been resolved
                
                // if invalid >= 50 {
                //     error!("received 50 packets that could not be segmented");
                //     warn!("you probably started capturing when you were already in-game");
                //     warn!("the capture needs to start on the main menu screen");
                //     warn!("log out then log back in");
                //     return None;
                // }
            } else {
                invalid = 0.max(invalid - 1);
                for command in commands {
                    exporter.read_command(command);
                }

                if exporter.is_finished() {
                    info!("retrieved all relevant packets, stop capturing");
                    break;
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
    let (tx, rx) = mpsc::channel();
    let mut join_handles = Vec::new();

    // we need to specify a specific network device when using pcap to capture network packets.
    // to lessen the burden on the user, we instead just capture *all* valid network devices
    // by capturing each on a different thread and sending the captured packets to a mpsc channel
    for device in Device::list()
        .unwrap()
        .into_iter()
        .filter(|d| matches!(d.flags.connection_status, ConnectionStatus::Connected))
        .filter(|d| !d.addresses.is_empty())
        .filter(|d| !d.flags.is_loopback())
    {
        let tx = tx.clone();
        let handle = std::thread::spawn(move || capture_device(device, tx));
        join_handles.push(handle);
    }

    // we clone tx into every thread, but at the end the original tx still remains.
    // rx.recv will continue to listen while at least one tx is still alive.
    // we drop the original tx to make sure that there are no tx alive after all threads
    // have dropped theirs
    drop(tx);

    let mut invalid = 0;
    // let mut warning_sent = false;

    info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");
    info!("listening with a timeout of {} seconds...", args.timeout);

    'recv: loop {
        match rx.recv_timeout(Duration::from_secs(args.timeout)) {
            Ok(data) => {
                match sniffer.receive_packet(data.to_vec()) {
                    Some(GamePacket::Connection(c)) => {
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
                    Some(GamePacket::Commands(commands)) => {
                        if commands.is_empty() {
                            invalid += 1;
                            
                            // FIXME: disable the invalid packet checks until the situation in
                            // reliquary lib has been resolved
                            
                            // if invalid >= 100 && !warning_sent {
                            //     error!(
                            //         "received a large number of packets that could not be parsed"
                            //     );
                            //     warn!(
                            //         "you probably started capturing when you were already in-game"
                            //     );
                            //     warn!("please log out and log back in");
                            //     warning_sent = true;
                            // }
                        } else {
                            invalid = 0.max(invalid - 1);

                            for command in commands {
                                if command.command_id == PlayerLoginScRsp {
                                    info!("detected login");
                                }

                                if command.command_id == PlayerLoginFinishScRsp {
                                    info!("detected login end, assume initialization is finished");
                                    break 'recv;
                                }

                                exporter.read_command(command);
                            }

                            if exporter.is_finished() {
                                info!("retrieved all relevant packets, stop listening");
                                break 'recv;
                            }
                        }
                    }
                    _ => {}
                }
            }
            Err(e) => {
                warn!(%e);
                break;
            }
        }
    }

    exporter.export()
}

#[instrument(skip_all, fields(device = device.desc))]
fn capture_device(device: Device, tx: mpsc::Sender<Vec<u8>>) {
    let mut capture = pcap::Capture::from_device(device)
        .unwrap()
        .immediate_mode(true)
        .promisc(true)
        .timeout(0) // explicitly disable timeout??
        .open()
        .unwrap();

    capture.filter(PACKET_FILTER, true).unwrap();

    debug!("listening");

    let mut has_captured = false;

    loop {
        match capture.next_packet() {
            Ok(packet) => {
                trace!("captured packet");
                if let Err(e) = tx.send(packet.data.to_vec()) {
                    debug!("channel closed: {e}");
                    break;
                }

                has_captured = true;
            }
            Err(e) => {
                // we only really care about capture errors on devices that we already know
                // are relevant (have sent packets before) and send those errors on warn level.
                //
                // if a capture errors right after initialization or on a device that did
                // not receive any relevant packets, error is less useful to the user,
                // so we lower the logging level

                if !has_captured {
                    debug!(?e);
                    break;
                } else if matches!(e, Error::TimeoutExpired) {
                    // somehow a timeout error can still happen even if i explicitly
                    // disable the timeout?? why :sob:
                    debug!(?e);
                    continue;
                } else {
                    warn!(?e);
                    break;
                }
            }
        }
    }

    debug!("stop listening");
}
