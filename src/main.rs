use std::fs::File;
use std::path::PathBuf;

use clap::Parser;
use reliquary::network::{ConnectionPacket, GamePacket, GameSniffer};
use reliquary::network::gen::command_id;
use reliquary::network::gen::proto::GetAvatarDataScRsp::GetAvatarDataScRsp;
use reliquary::network::gen::proto::GetBagScRsp::GetBagScRsp;
use reliquary::network::gen::proto::GetHeroBasicTypeInfoScRsp::GetHeroBasicTypeInfoScRsp;
use reliquary::network::gen::proto::PlayerGetTokenScRsp::PlayerGetTokenScRsp;
use tracing::info;

use reliquary_archiver::export::optimizer::ExportForOptimizer;

#[derive(Parser, Debug)]
struct Args {
    output: PathBuf,
    #[arg(long)]
    pcap: Option<PathBuf>,
}

fn main() {
    tracing_subscriber::fmt::init();

    let args = Args::parse();

    match args.pcap {
        Some(_) => {
            file_capture(args);
        }
        None => {
            live_capture(args);
        }
    }
}

fn file_capture(args: Args) {
    info!("start reading pcap");

    let mut capture = pcap::Capture::from_file(args.pcap.unwrap())
        .expect("could not read pcap file");

    capture.filter("udp portrange 23301-23302", false).unwrap();

    let mut sniffer = GameSniffer::new();
    let mut exporter = ExportForOptimizer::new_from_online();
    while let Ok(packet) = capture.next_packet() {
        match sniffer.receive_packet(packet.data.to_vec()) {
            Some(GamePacket::Connection(ConnectionPacket::Disconnected)) => {
                info!("disconnected");
                break;
            }
            Some(GamePacket::Commands(commands)) => {
                for command in commands {
                    match command.command_id {
                        command_id::PlayerGetTokenScRsp => {
                            exporter.write_uid(
                                command.parse_proto::<PlayerGetTokenScRsp>().unwrap().uid.to_string()
                            )
                        }
                        command_id::GetBagScRsp => {
                            exporter.write_bag(
                                command.parse_proto::<GetBagScRsp>().unwrap()
                            )
                        }
                        command_id::GetAvatarDataScRsp => {
                            exporter.write_characters(
                                command.parse_proto::<GetAvatarDataScRsp>().unwrap()
                            )
                        }
                        command_id::GetHeroBasicTypeInfoScRsp => {
                            exporter.write_hero(
                                command.parse_proto::<GetHeroBasicTypeInfoScRsp>().unwrap()
                            )
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    info!("end reading pcap");

    info!("exporting...");

    let file = File::create(&args.output).unwrap();
    serde_json::to_writer_pretty(&file, exporter.export()).unwrap();

    info!("wrote output to {}", &args.output.display());
}

fn live_capture(_args: Args) {
    todo!()
}