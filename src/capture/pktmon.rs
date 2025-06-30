use std::{sync::atomic::Ordering, time::Duration};

use super::*;
use futures::{executor::block_on, SinkExt};
use ::pktmon::{Capture, filter::{PktMonFilter, TransportProtocol}};

pub struct PktmonBackend;

#[derive(Debug)]
pub struct PktmonCaptureDevice;

pub struct PktmonCapture {
    capture: Capture,
}

impl CaptureBackend for PktmonBackend {
    type Device = PktmonCaptureDevice;
    
    fn list_devices(&self) -> Result<Vec<Self::Device>> {
        // PktMon doesn't need device selection - it captures all interfaces
        Ok(vec![PktmonCaptureDevice])
    }
}

impl CaptureDevice for PktmonCaptureDevice {
    type Capture = PktmonCapture;

    fn name(&self) -> &str {
        "pktmon"
    }
    
    fn create_capture(&self) -> Result<Self::Capture> {
        let mut capture = Capture::new()
            .map_err(|e| CaptureError::CaptureError { has_captured: false, error: Box::new(e) })?;

        let filter = PktMonFilter {
            name: "UDP Filter".to_string(),
            transport_protocol: Some(TransportProtocol::UDP),
            port: PORT_RANGE.0.into(),
            ..PktMonFilter::default()
        };
        
        capture.add_filter(filter)
            .map_err(|e| CaptureError::FilterError(Box::new(e)))?;

        let filter = PktMonFilter {
            name: "UDP Filter".to_string(),
            transport_protocol: Some(TransportProtocol::UDP),
            port: PORT_RANGE.1.into(),
            ..PktMonFilter::default()
        };
        
        capture.add_filter(filter)
            .map_err(|e| CaptureError::FilterError(Box::new(e)))?;
            
        Ok(PktmonCapture { capture })
    }
}

impl PacketCapture for PktmonCapture {
    #[instrument(skip_all)]
    fn capture_packets(mut self) -> Result<impl Stream<Item = Result<Packet>>> {
        let mut has_captured = false;

        self.capture.start()
            .map_err(|e| CaptureError::CaptureError { has_captured, error: Box::new(e) })?;

        return match self.capture.stream() {
            Ok(stream) => Ok(stream.map(|p| Ok(Packet { data: p.payload.to_vec() }))),
            Err(e) => Err(CaptureError::CaptureError { has_captured: false, error: Box::new(e) }),
        }

        // while !abort_signal.load(Ordering::Relaxed) {
        //     match self.capture.next_packet_timeout(Duration::from_secs(1)) {
        //         Ok(packet) => {
        //             let packet = Packet {
        //                 data: packet.payload.to_vec(),
        //             };
            
        //             block_on(tx.send(packet)).map_err(|_| CaptureError::ChannelClosed)?;
        //             has_captured = true;
        //         }
        //         Err(e) => {
        //             if matches!(e, RecvTimeoutError::Timeout) {
        //                 continue;
        //             }

        //             return Err(CaptureError::CaptureError { has_captured, error: Box::new(e) });
        //         }
        //     }
        // }

        // Ok(())
    }
}
