use std::future;

use ::pktmon::filter::{PktMonFilter, TransportProtocol};
use ::pktmon::{Capture, PacketPayload};
use futures::StreamExt;
use tracing::instrument;

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
