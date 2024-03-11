use std::env;
use std::path::Path;

use tracing::{info, instrument};

use reliquary::sniffer::gen::proto::GetBagScRsp::GetBagScRsp;
use reliquary::sniffer::{GameCommand, GamePacket, GameSniffer};
use reliquary::sniffer::connection::ConnectionPacket;

fn main() {
    tracing_subscriber::fmt::init();

    let args: Vec<String> = env::args().collect();
    let path = args.get(1).expect("not enough arguments, usage: from_pcap <file>");
    let path = Path::new(path.as_str());

    info!("start reading pcap");

    let mut capture = pcap::Capture::from_file(path).expect("could not read pcap file");
    capture.filter("udp portrange 23301-23302", false).unwrap();

    let mut sniffa = GameSniffer::new();

    while let Ok(packet) = capture.next_packet() {
        match sniffa.receive_packet(packet.data.to_vec()) {
            Some(GamePacket::Connection(ConnectionPacket::Disconnected)) => {
                info!("disconnected");
                break;
            }
            Some(GamePacket::Commands(commands)) => {
                for command in commands {
                    handle_command(command);
                }
            }
            _ => {}
        }
    }

    info!("end reading pcap");
}

#[instrument]
fn handle_command(command: GameCommand) {
    if let Some("GetBagScRsp") = command.get_command_name() {
        let msg = command.parse_proto::<GetBagScRsp>().unwrap();
        info!(?msg);
    }
}