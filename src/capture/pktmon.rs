use std::future;
use std::path::PathBuf;

use ::pktmon::filter::{PktMonFilter, TransportProtocol};
use ::pktmon::{Capture, EtlCapture, PacketPayload};
use futures::StreamExt;
use tracing::{info, instrument, warn};

use super::*;

pub struct PktmonBackend;

#[derive(Debug)]
pub struct PktmonCaptureDevice;

impl DeviceBackend for PktmonBackend {
    type Device = PktmonCaptureDevice;

    fn list_devices(&self) -> Result<Vec<Self::Device>> {
        // PktMon doesn't need device selection - it captures all interfaces
        Ok(vec![PktmonCaptureDevice])
    }

    fn device_name(_device: &Self::Device) -> String {
        "pktmon".to_string()
    }

    #[instrument(skip_all)]
    fn open_device(_device: &Self::Device) -> Result<PacketStream> {
        let mut capture = Capture::new().map_err(|e| CaptureError::Capture(Box::new(e)))?;

        for port in [PORT_RANGE.0, PORT_RANGE.1] {
            let filter = PktMonFilter {
                name: "UDP Filter".to_string(),
                transport_protocol: Some(TransportProtocol::UDP),
                port: port.into(),
                ..PktMonFilter::default()
            };

            capture.add_filter(filter).map_err(|e| CaptureError::Filter(Box::new(e)))?;
        }

        capture.start().map_err(|e| CaptureError::Capture(Box::new(e)))?;

        let stream = capture.stream().map_err(|e| CaptureError::Capture(Box::new(e)))?;

        Ok(stream
            .filter_map(|packet| {
                future::ready(match packet.payload {
                    PacketPayload::Ethernet(payload) => Some(Ok(Packet {
                        source_id: packet.component_id as u64,
                        data: payload,
                    })),
                    _ => None,
                })
            })
            .boxed())
    }
}

/// Reads packets from a recorded .etl file
pub struct EtlFileSource {
    path: PathBuf,
}

impl EtlFileSource {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }
}

impl PacketSource for EtlFileSource {
    #[instrument(skip_all)]
    fn open(self: Box<Self>) -> Result<PacketStream> {
        info!("Capturing from etl file: {}", self.path.display());
        let mut capture = EtlCapture::new(&self.path).map_err(|e| CaptureError::Device(Box::new(e)))?;
        capture.start().map_err(|e| CaptureError::Capture(Box::new(e)))?;

        let source_id = file_source_id(&self.path);

        Ok(blocking_stream(move |send| {
            while let Ok(packet) = capture.next_packet() {
                if let PacketPayload::Ethernet(data) = packet.payload {
                    if !send(Packet { source_id, data }) {
                        break;
                    }
                }
            }

            if let Err(e) = capture.stop() {
                warn!(%e, "could not stop etl capture");
            }
        }))
    }
}
