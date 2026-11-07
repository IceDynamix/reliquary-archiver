use std::future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use ::pcap::{self, Capture, Device as PcapDevice};
use futures::StreamExt;
use pcap::PacketCodec;
use tracing::{info, instrument, warn};

use super::*;

pub struct PcapBackend;

impl DeviceBackend for PcapBackend {
    type Device = PcapDevice;

    fn list_devices(&self) -> Result<Vec<Self::Device>> {
        Ok(PcapDevice::list()
            .map_err(|e| CaptureError::Device(Box::new(e)))?
            .into_iter()
            .filter(|d| matches!(d.flags.connection_status, pcap::ConnectionStatus::Connected))
            .filter(|d| !d.addresses.is_empty())
            .filter(|d| !d.flags.is_loopback())
            .collect::<Vec<_>>())
    }

    fn device_name(device: &Self::Device) -> String {
        device.name.clone()
    }

    #[instrument(skip_all, fields(device = device.desc))]
    fn open_device(device: &Self::Device) -> Result<PacketStream> {
        let capture = Capture::from_device(device.clone())
            .map_err(|e| CaptureError::Device(Box::new(e)))?
            .immediate_mode(true)
            .promisc(true)
            .timeout(1000)
            .buffer_size(1024 * 1024 * 16) // 16MB
            .open()
            .map_err(|e| CaptureError::Capture(Box::new(e)))?;

        let mut capture = capture.setnonblock().map_err(|e| CaptureError::Capture(Box::new(e)))?;

        capture.filter(PCAP_FILTER, true).map_err(|e| CaptureError::Filter(Box::new(e)))?;

        let mut hasher = DefaultHasher::new();
        device.name.hash(&mut hasher);
        let source_id = hasher.finish();

        let device_name = device.name.clone();
        let stream = capture
            .stream(Codec { source_id })
            .map_err(|e| CaptureError::Capture(Box::new(e)))?;

        Ok(stream
            .take_while(move |r| {
                future::ready(match r {
                    // Check if this is a device removal error
                    Err(pcap::Error::PcapError(error_msg)) if error_msg.contains("ERROR_DEVICE_REMOVED") => {
                        warn!(%error_msg, %device_name, "Device removed, terminating capture stream");
                        false // Stop the stream immediately
                    }
                    _ => true, // Continue for Ok packets and other errors
                })
            })
            .map(|r| r.map_err(|e| CaptureError::Capture(Box::new(e))))
            .boxed())
    }
}

pub struct Codec {
    source_id: u64,
}

impl PacketCodec for Codec {
    type Item = Packet;

    fn decode(&mut self, pkt: pcap::Packet) -> Self::Item {
        Packet {
            source_id: self.source_id,
            data: pkt.data.to_vec(),
        }
    }
}

/// Reads packets from a recorded .pcap file
pub struct PcapFileSource {
    path: PathBuf,
}

impl PcapFileSource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl PacketSource for PcapFileSource {
    #[instrument(skip_all)]
    fn open(self: Box<Self>) -> Result<PacketStream> {
        info!("Capturing from pcap file: {}", self.path.display());
        let mut capture = Capture::from_file(&self.path).map_err(|e| CaptureError::Device(Box::new(e)))?;
        let linktype = capture.get_datalink();
        capture.filter(PCAP_FILTER, false).map_err(|e| CaptureError::Filter(Box::new(e)))?;

        let source_id = file_source_id(&self.path);

        Ok(blocking_stream(move |send| {
            while let Ok(packet) = capture.next_packet() {
                let Ok(data) = normalize_offline_pcap_payload(linktype, packet.data) else {
                    continue;
                };

                if !send(Packet { source_id, data }) {
                    break;
                }
            }
        }))
    }
}
