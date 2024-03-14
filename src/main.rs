use std::fs::File;
use std::path::PathBuf;

use clap::Parser;
use reliquary::network::{ConnectionPacket, GamePacket, GameSniffer};
use tracing::{info, instrument};

use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::optimizer::{Database, OptimizerExporter};

const PACKET_FILTER: &str = "udp portrange 23301-23302";

#[derive(Parser, Debug)]
struct Args {
    /// Path to output .json file to
    output: PathBuf,
    /// Read packets from .pcap file instead of capturing live packets
    #[arg(long)]
    pcap: Option<PathBuf>,
}

fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();
    info!("{args:?}");

    let database = Database::new_from_online();
    let sniffer = GameSniffer::new().set_initial_keys(database.keys().clone());
    let exporter = OptimizerExporter::new(database);

    let export = match args.pcap {
        Some(_) => file_capture(&args, exporter, sniffer),
        None => live_capture(&args, exporter, sniffer)
    };

    let file = File::create(&args.output).unwrap();
    serde_json::to_writer_pretty(&file, &export).unwrap();

    info!("wrote output to {}", &args.output.display());
}

#[instrument(skip_all)]
fn file_capture<E>(args: &Args, mut exporter: E, mut sniffer: GameSniffer) -> E::Export
    where E: Exporter
{
    let mut capture = pcap::Capture::from_file(args.pcap.as_ref().unwrap())
        .expect("could not read pcap file");

    capture.filter(PACKET_FILTER, false).unwrap();

    while let Ok(packet) = capture.next_packet() {
        match sniffer.receive_packet(packet.data.to_vec()) {
            Some(GamePacket::Connection(ConnectionPacket::Disconnected)) => {
                info!("disconnected");
                break;
            }
            Some(GamePacket::Commands(commands)) => {
                for command in commands {
                    exporter.read_command(command);
                }

                if exporter.is_finished() {
                    info!("retrieved all relevant packets, stop reading capture");
                    break;
                }
            }
            _ => {}
        }
    }

    exporter.export()
}

#[instrument(skip_all)]
fn live_capture<E>(_args: &Args, _exporter: E, _sniffer: GameSniffer) -> E::Export
    where E: Exporter
{
    todo!()
}