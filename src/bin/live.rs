use std::sync::mpsc;
use std::sync::mpsc::{Receiver, Sender};
use std::time::Duration;

use pcap::{ConnectionStatus, Device};
use tracing::{info, Level, span, trace, warn};

use reliquary::sniffer::{GamePacket, GameSniffer};
use reliquary::sniffer::connection::ConnectionPacket;

fn main() {
    tracing_subscriber::fmt::init();

    info!("start sniffing");

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

    handle_channel(rx);

    for handle in join_handles {
        handle.join().unwrap();
    }

    info!("end sniffing");
}

fn capture_device(device: Device, tx: Sender<Vec<u8>>) {
    let span = span!(Level::INFO, "pcap", device=device.name);
    let _enter = span.enter();

    let mut capture = pcap::Capture::from_device(device)
        .unwrap()
        .immediate_mode(true)
        .open()
        .unwrap();

    capture.filter("udp portrange 23301-23302", false).unwrap();

    while let Ok(packet) = capture.next_packet() {
        trace!("captured packet");
        if let Err(e) = tx.send(packet.data.to_vec()) {
            warn!("channel closed: {e}");
            return;
        }
    }
}

fn handle_channel(rx: Receiver<Vec<u8>>) {
    let span = span!(Level::INFO, "channel");
    let _enter = span.enter();

    let mut sniffa = GameSniffer::new();

    loop {
        match rx.recv_timeout(Duration::from_secs(30)) {
            Ok(data) => {
                match sniffa.receive_packet(data.to_vec()) {
                    Some(GamePacket::Connection(ConnectionPacket::Disconnected)) => {
                        info!("disconnected");
                        break;
                    }
                    Some(GamePacket::Commands(commands)) => {
                        for command in commands {
                            info!("do something with me");
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
}

