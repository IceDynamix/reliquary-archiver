use std::{hash::{DefaultHasher, Hash, Hasher}, sync::atomic::Ordering};

use super::*;
use futures::{executor::block_on, SinkExt, StreamExt, TryStreamExt};
use pcap::PacketCodec;
use ::pcap::{self, Active, Device as PcapDevice, Capture};
use tracing::{debug, instrument};

pub struct PcapBackend;

pub struct PcapCapture {
    capture: Capture<Active>,
    device: PcapDevice,
    id: u64,
}

impl CaptureBackend for PcapBackend {
    type Device = PcapDevice;
    
    fn list_devices(&self) -> Result<Vec<Self::Device>> {
        Ok(PcapDevice::list()
            .map_err(|e| CaptureError::DeviceError(Box::new(e)))?
            .into_iter()
            .filter(|d| matches!(d.flags.connection_status, pcap::ConnectionStatus::Connected))
            .filter(|d| !d.addresses.is_empty())
            .filter(|d| !d.flags.is_loopback())
            .collect::<Vec<_>>())
    }
}

impl CaptureDevice for PcapDevice {
    type Capture = PcapCapture;

    fn name(&self) -> &str {
        &self.name
    }
    
    fn create_capture(&self) -> Result<Self::Capture> {
        let mut capture = Capture::from_device(self.clone())
            .map_err(|e| CaptureError::DeviceError(Box::new(e)))?
            .immediate_mode(true)
            .promisc(true)
            .timeout(1000)
            .buffer_size(1024 * 1024 * 16) // 16MB
            .open()
            .map_err(|e| CaptureError::CaptureError { has_captured: false, error: Box::new(e) })?;

        let mut capture = capture
            .setnonblock()
            .map_err(|e| CaptureError::CaptureError { has_captured: false, error: Box::new(e) })?;

        capture.filter(PCAP_FILTER, true)
            .map_err(|e| CaptureError::FilterError(Box::new(e)))?;

        let mut hasher = DefaultHasher::new();
        self.name.hash(&mut hasher);
        let id = hasher.finish();
            
        Ok(PcapCapture { capture, device: self.clone(), id })
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

impl PacketCapture for PcapCapture {
    #[instrument(skip_all, fields(device = self.device.desc))]
    fn capture_packets(mut self) -> Result<impl Stream<Item = Result<Packet>>> {
        let mut has_captured = false;
        return match self.capture.stream(Codec { source_id: self.id }) {
            Ok(stream) => Ok(stream
                .map(move |r| match r {
                    Ok(p) => {
                        has_captured = true;
                        Ok(p)
                    }
                    Err(e) => return Err(CaptureError::CaptureError { has_captured, error: Box::new(e) }),
                })),
            Err(e) => Err(CaptureError::CaptureError { has_captured: false, error: Box::new(e) }),
        }
    }
} 

