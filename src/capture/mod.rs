use std::error::Error;
use std::fmt::{Debug, Display};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use futures::{Stream, StreamExt};
use tokio::pin;
use tokio::sync::mpsc;
use tokio_stream::StreamMap;
use tracing::instrument;

use crate::scopefns::Also;

#[cfg(all(windows, feature = "pktmon"))]
pub mod pktmon;

#[cfg(feature = "pcap")]
pub mod pcap;

#[cfg(not(any(feature = "pktmon", feature = "pcap")))]
compile_error!("at least one of the features \"pktmon\" or \"pcap\" must be enabled");

#[cfg(feature = "pktmon")]
pub const PORT_RANGE: (u16, u16) = (23301, 23302);

#[cfg(feature = "pcap")]
pub const PCAP_FILTER: &str = "udp portrange 23301-23302";

#[derive(Debug)]
pub enum CaptureError {
    #[cfg(feature = "pcap")]
    DeviceError(Box<dyn Error + Send + Sync>),

    FilterError(Box<dyn Error + Send + Sync>),
    CaptureError {
        has_captured: bool,
        error: Box<dyn Error + Send + Sync>,
    },
}

impl Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source().unwrap())
    }
}

impl Error for CaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(feature = "pcap")]
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
    pub source_id: u64,
    pub data: Vec<u8>,
}

/// Trait for implementing different packet capture backends
pub trait PacketCapture: Send {
    /// Start capturing packets and send them through the channel
    fn capture_packets(self) -> Result<impl Stream<Item = Result<Packet>> + Unpin + Send>;
}

/// Trait for creating packet capture instances
pub trait CaptureDevice: Send + Debug {
    type Capture: PacketCapture;

    /// Get the name of the device
    fn name(&self) -> &str;

    /// Create a new capture instance from this device
    fn create_capture(&self) -> Result<Self::Capture>;
}

/// Get all available capture devices for a specific backend
pub trait CaptureBackend: Send {
    type Device: CaptureDevice;

    /// List all available capture devices
    fn list_devices(&self) -> Result<Vec<Self::Device>>;
}

/// Start capturing packets from all available devices using the specified backend
#[instrument(skip_all)]
pub fn listen_on_all<B: CaptureBackend + 'static>(backend: B) -> Result<impl Stream<Item = Result<Packet>> + Unpin> {
    use std::collections::HashSet;

    use tokio::time::{Duration, interval};

    let devices = backend.list_devices()?;
    let mut merged_stream = StreamMap::new();

    if devices.is_empty() {
        tracing::warn!("Could not find any network devices");
    }

    // Create a channel to forward packets
    let (tx, rx) = mpsc::unbounded_channel();

    // Spawn task to manage stream and discover new devices
    tokio::spawn(async move {
        let mut check_interval = interval(Duration::from_secs(10));

        loop {
            tokio::select! {
                // Forward packets from merged stream
                Some((_, item)) = merged_stream.next() => {
                    if let Err(e) = &item {
                        tracing::warn!(%e);
                    }

                    if tx.send(item).is_err() {
                        break; // Receiver dropped
                    }
                }

                // Check for new devices every 10 seconds
                _ = check_interval.tick() => {
                    match backend.list_devices() {
                        Ok(devices) => {
                            for device in devices {
                                let device_name = device.name().to_owned();

                                // Only add if it's a new device
                                if !merged_stream.contains_key(&device_name) {
                                    tracing::info!("Discovered new device: {}", device_name);

                                    match device.create_capture() {
                                        Ok(mut capture) => {
                                            match capture.capture_packets() {
                                                Ok(stream) => {
                                                    merged_stream.insert(device_name, stream);
                                                }
                                                Err(e) => {
                                                    tracing::warn!("Capture initialization error on new device {}: {:#?}", device_name, e);
                                                }
                                            }
                                        }
                                        Err(e) => {
                                            tracing::warn!("Failed to create capture for new device {}: {:#?}", device_name, e);
                                        }
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            tracing::warn!("Failed to list devices during periodic check: {:#?}", e);
                        }
                    }
                }
            }
        }
    });

    Ok(tokio_stream::wrappers::UnboundedReceiverStream::new(rx))
}
