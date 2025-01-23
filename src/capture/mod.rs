use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::sync::{mpsc, Arc};
use std::fmt::{Debug, Display};
use std::thread::JoinHandle;

use tracing::instrument;

#[cfg(windows)] pub mod pktmon;
pub mod pcap;

pub const PORT_RANGE: (u16, u16) = (23301, 23302);
pub const PCAP_FILTER: &str = "udp portrange 23301-23302";

#[derive(Debug)]
pub enum CaptureError {
    DeviceError(Box<dyn Error>),
    FilterError(Box<dyn Error>),
    CaptureError { has_captured: bool, error: Box<dyn Error> },
    ChannelClosed,
}

impl Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if matches!(self, CaptureError::ChannelClosed) {
            write!(f, "Channel closed")
        } else {
            write!(f, "{}", self.source().unwrap().to_string())
        }
    }
}

impl Error for CaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            CaptureError::DeviceError(e) => Some(e.as_ref()),
            CaptureError::FilterError(e) => Some(e.as_ref()),
            CaptureError::CaptureError { error, .. } => Some(error.as_ref()),
            _ => None,
        }
    }
}

pub type Result<T> = std::result::Result<T, CaptureError>;

/// Represents a captured network packet
#[derive(Debug, Clone)]
pub struct Packet {
    pub data: Vec<u8>,
}

/// Trait for implementing different packet capture backends
pub trait PacketCapture: Send {
    /// Start capturing packets and send them through the channel
    fn capture_packets(
        &mut self, 
        tx: mpsc::Sender<Packet>,
        abort_signal: Arc<AtomicBool>,
    ) -> Result<()>;
}

/// Trait for creating packet capture instances
pub trait CaptureDevice: Send + Debug {
    type Capture: PacketCapture;
    
    /// Create a new capture instance from this device
    fn create_capture(&self) -> Result<Self::Capture>;
}

/// Get all available capture devices for a specific backend
pub trait CaptureBackend {
    type Device: CaptureDevice;
    
    /// List all available capture devices
    fn list_devices() -> Result<Vec<Self::Device>>;
}

/// Start capturing packets from all available devices using the specified backend
#[instrument(skip_all)]
pub fn listen_on_all<B: CaptureBackend + 'static>(
    abort_signal: Arc<AtomicBool>,
) -> Result<(mpsc::Receiver<Packet>, Vec<JoinHandle<()>>)> {
    let (tx, rx) = mpsc::channel();
    
    let devices = B::list_devices()?;
    let mut join_handles = Vec::new();
    
    for device in devices {
        let tx = tx.clone();
        let abort_signal = abort_signal.clone();

        join_handles.push(std::thread::spawn(move || {
            tracing::debug!("Starting capture thread for device: {:?}", device);

            let mut capture = match device.create_capture() {
                Ok(c) => c,
                Err(e) => {
                    tracing::debug!("Failed to create capture: {:#?}", e);
                    return;
                }
            };
            
            if let Err(e) = capture.capture_packets(tx, abort_signal) {
                match e {
                    CaptureError::ChannelClosed => {
                        tracing::debug!("Channel closed");
                    }
                    CaptureError::CaptureError { has_captured, error } => {
                        // we only really care about capture errors on devices that we already know
                        // are relevant (have sent packets before) and send those errors on warn level.
                        //
                        // if a capture errors right after initialization or on a device that did
                        // not receive any relevant packets, error is less useful to the user,
                        // so we lower the logging level
                        if !has_captured {
                            tracing::debug!("Capture error: {:#?}", error);
                        } else {
                            tracing::warn!("Capture error: {:#?}", error);
                        }
                    }
                    _ => {
                        tracing::warn!("Unexpected non-capture error: {:#?}", e);
                    }
                }
            }

            tracing::debug!("Capture thread finished");
        }));
    }

    Ok((rx, join_handles))
}
