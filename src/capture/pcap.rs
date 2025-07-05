use std::{hash::{DefaultHasher, Hash, Hasher}, sync::atomic::Ordering};

use super::*;
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
    
    fn create_capture(&self) -> Result<Self::Capture> {
        let mut capture = Capture::from_device(self.clone())
            .map_err(|e| CaptureError::DeviceError(Box::new(e)))?
            .immediate_mode(true)
            .promisc(true)
            .timeout(1000)
            .open()
            .map_err(|e| CaptureError::CaptureError { has_captured: false, error: Box::new(e) })?;

        capture.filter(PCAP_FILTER, true)
            .map_err(|e| CaptureError::FilterError(Box::new(e)))?;

        let mut hasher = DefaultHasher::new();
        self.name.hash(&mut hasher);
        let id = hasher.finish();
            
        Ok(PcapCapture { capture, device: self.clone(), id })
    }
}

impl PacketCapture for PcapCapture {
    #[instrument(skip_all, fields(device = self.device.desc))]
    fn capture_packets(&mut self, tx: mpsc::Sender<Packet>, abort_signal: Arc<AtomicBool>) -> Result<()> {
        let mut has_captured = false;

        while !abort_signal.load(Ordering::Relaxed) {
            match self.capture.next_packet() {
                Ok(packet) => {
                    let packet = Packet {
                        source_id: self.id,
                        data: packet.data.to_vec(),
                    };

                    tx.send(packet).map_err(|_| CaptureError::ChannelClosed)?;
                    has_captured = true;
                }
                Err(e) => {
                    if matches!(e, pcap::Error::TimeoutExpired) {
                        debug!(?e);
                        continue;
                    }

                    return Err(CaptureError::CaptureError { has_captured, error: Box::new(e) });
                }
            }
        }

        Ok(())
    }
} 

