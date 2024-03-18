use std::fs::File;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

use clap::Parser;
use pcap::{ConnectionStatus, Device};
use reliquary::network::{GamePacket, GameSniffer};
use tracing::{debug, error, info, instrument, trace, warn};
use tracing_subscriber::EnvFilter;

use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::optimizer::{Database, OptimizerExporter};

const PACKET_FILTER: &str = "udp portrange 23301-23302";

#[derive(Parser, Debug)]
struct Args {
    #[arg(default_value = "archive_output.json")]
    /// Path to output .json file to
    output: PathBuf,
    /// Read packets from .pcap file instead of capturing live packets
    #[arg(long)]
    pcap: Option<PathBuf>,
    /// How long to wait in seconds until timeout is triggered (for live capture)
    #[arg(long, default_value_t = 120)]
    timeout: u64,
    /// How verbose the output should be, can be set up to 3 times. Has no effect if RUST_LOG is set
    #[arg(short, long, action = clap::ArgAction::Count)]
    verbose: u8,
}

fn main() {
    color_eyre::install().unwrap();
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                .with_default_directive(
                    match args.verbose {
                        0 => "reliquary_archiver=info",
                        1 => "info",
                        2 => "debug",
                        _ => "trace"
                    }.parse().unwrap()
                )
                .from_env_lossy()
        )
        .init();

    debug!(?args);

    let database = Database::new_from_online();
    let sniffer = GameSniffer::new().set_initial_keys(database.keys().clone());
    let exporter = OptimizerExporter::new(database);

    let export = match args.pcap {
        Some(_) => file_capture(&args, exporter, sniffer),
        None => live_capture(&args, exporter, sniffer)
    };

    if let Some(export) = export {
        let file = File::create(&args.output).unwrap();
        serde_json::to_writer_pretty(&file, &export).unwrap();
        info!("wrote output to {}", &args.output.display());
    }

    info!("press enter to close");
    std::io::stdin().read_line(&mut String::new()).unwrap();
}

#[instrument(skip_all)]
fn file_capture<E>(args: &Args, mut exporter: E, mut sniffer: GameSniffer) -> Option<E::Export>
    where E: Exporter
{
    let mut capture = pcap::Capture::from_file(args.pcap.as_ref().unwrap())
        .expect("could not read pcap file");

    capture.filter(PACKET_FILTER, false).unwrap();

    let mut invalid = 0;

    info!("capturing");
    while let Ok(packet) = capture.next_packet() {
        if let Some(GamePacket::Commands(commands)) = sniffer.receive_packet(packet.data.to_vec()) {
            if commands.is_empty() {
                invalid += 1;

                if invalid >= 50 {
                    error!("received 50 packets that could not be segmented");
                    warn!("you probably started capturing when you were already in-game");
                    warn!("the capture needs to start on the main menu screen before hyperdrive");
                    return None;
                }
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

    Some(exporter.export())
}

#[instrument(skip_all)]
fn live_capture<E>(args: &Args, mut exporter: E, mut sniffer: GameSniffer) -> Option<E::Export>
    where E: Exporter
{
    let (tx, rx) = mpsc::channel();
    let mut join_handles = Vec::new();

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

    let mut invalid = 0;
    let mut warning_sent = false;

    info!("instructions: go to main menu screen and go into train hyperdrive");
    info!("listening with a timeout of {} seconds...", args.timeout);

    loop {
        match rx.recv_timeout(Duration::from_secs(args.timeout)) {
            Ok(data) => {
                if let Some(GamePacket::Commands(commands)) = sniffer.receive_packet(data.to_vec()) {
                    if commands.is_empty() {
                        invalid += 1;

                        if invalid >= 25 && !warning_sent {
                            error!("received a large number of packets that could not be parsed");
                            warn!("you probably started capturing when you were already in-game");
                            warn!("please log out and log back in");
                            warning_sent = true;
                        }
                    } else {
                        invalid -= 10;

                        for command in commands {
                            exporter.read_command(command);
                        }

                        if exporter.is_finished() {
                            info!("retrieved all relevant packets, stop listening");
                            break;
                        }
                    }
                }
            }
            Err(e) => {
                warn!(%e);
                break;
            }
        }
    }

    Some(exporter.export())
}

#[instrument("thread", skip_all, fields(device = device.name))]
fn capture_device(device: Device, tx: mpsc::Sender<Vec<u8>>) {
    let mut capture = pcap::Capture::from_device(device)
        .unwrap()
        .immediate_mode(true)
        .open()
        .unwrap();

    capture.filter(PACKET_FILTER, false).unwrap();

    debug!("listening");

    while let Ok(packet) = capture.next_packet() {
        trace!("captured packet");
        if let Err(e) = tx.send(packet.data.to_vec()) {
            debug!("channel closed: {e}");
            return;
        }
    }
}

