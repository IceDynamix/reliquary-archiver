use std::sync::atomic::Ordering;

use super::*;

pub struct EtlBackend {
    etl_path: String,
}

#[derive(Debug)]
pub struct EtlCaptureDevice {
    etl_path: String,
}

pub struct EtlCapture {
    capture: ::pktmon::EtlCapture,
}

impl CaptureBackend for EtlBackend {
    type Device = EtlCaptureDevice;
    
    fn list_devices(&self) -> Result<Vec<Self::Device>> {
        // Etl doesn't need device selection - just reading from a file
        Ok(vec![EtlCaptureDevice { etl_path: self.etl_path.clone() }])
    }
}

impl CaptureDevice for EtlCaptureDevice {
    type Capture = EtlCapture;
    
    fn create_capture(&self) -> Result<Self::Capture> {
        let capture = ::pktmon::EtlCapture::new(self.etl_path.clone())
            .map_err(|e| CaptureError::CaptureError { has_captured: false, error: Box::new(e) })?;

        Ok(EtlCapture { capture })
    }
}

impl PacketCapture for EtlCapture {
    #[instrument(skip_all)]
    fn capture_packets(&mut self, tx: mpsc::Sender<Packet>, abort_signal: Arc<AtomicBool>) -> Result<()> {
        let mut has_captured = false;

        self.capture.start()
            .map_err(|e| CaptureError::CaptureError { has_captured, error: Box::new(e) })?;

        while !abort_signal.load(Ordering::Relaxed) {
            match self.capture.next_packet() {
                Ok(packet) => {
                    let packet = Packet {
                        data: packet.payload.to_vec(),
                    };
            
                    tx.send(packet).map_err(|_| CaptureError::ChannelClosed)?;
                    has_captured = true;
                }
                Err(e) => {
                    return Err(CaptureError::CaptureError { has_captured, error: Box::new(e) });
                }
            }
        }

        Ok(())
    }
}
