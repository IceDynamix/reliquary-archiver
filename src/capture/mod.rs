use std::collections::HashSet;
use std::error::Error;
use std::fmt::{Debug, Display};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use async_stream::stream;
use futures::stream::BoxStream;
use futures::{Stream, StreamExt};
use tokio::pin;
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio_stream::StreamMap;
use tracing::instrument;

use crate::scopefns::Also;

#[cfg(all(windows, feature = "pktmon"))]
pub mod pktmon;

#[cfg(feature = "pcap")]
pub mod pcap;
#[cfg(feature = "pcap-parser")]
pub mod pcap_file;

#[cfg(not(any(feature = "pktmon", feature = "pcap", feature = "pcap-parser")))]
compile_error!("at least one of the features \"pktmon\" or \"pcap\" must be enabled");

#[cfg(feature = "pktmon")]
pub const PORT_RANGE: (u16, u16) = (23301, 23302);

#[cfg(feature = "pcap")]
pub const PCAP_FILTER: &str = "udp portrange 23301-23302";

#[derive(Debug)]
pub enum CaptureError {
    #[cfg(any(feature = "pcap", feature = "pcap-parser"))]
    Device(Box<dyn Error + Send + Sync>),

    Filter(Box<dyn Error + Send + Sync>),
    Capture {
        has_captured: bool,
        error: Box<dyn Error + Send + Sync>,
    },
}

impl Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(source) = self.source() {
            write!(f, "{}", source)
        } else {
            write!(f, "None")
        }
    }
}

impl Error for CaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            #[cfg(any(feature = "pcap", feature = "pcap-parser"))]
            CaptureError::Device(e) => Some(e.as_ref()),
            CaptureError::Filter(e) => Some(e.as_ref()),
            CaptureError::Capture { error, .. } => Some(error.as_ref()),
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
    fn list_devices(&mut self) -> Result<Vec<Self::Device>>;

    fn listen_new_devices(&mut self) -> BoxStream<'_, Result<Self::Device>> {
        let mut set = HashSet::new();
        stream! {
            let mut check_interval = interval(Duration::from_secs(10));
            loop {
                match self.list_devices() {
                    Ok(devices) => {
                        set.retain(|device| {
                            for d in &devices {
                                if device == d.name() {
                                    return true;
                                }
                            }
                            false
                        });
                        for device in devices {
                            let name = device.name().to_string();
                            if !set.contains(name.as_str()) {
                                set.insert(name);
                                yield Ok(device);
                            }
                        }
                    }
                    Err(err) => yield Err(err),
                }
                check_interval.tick().await;
            }
        }
        .boxed()
    }
}

/// Start capturing packets from all available devices using the specified backend
#[instrument(skip_all)]
pub fn listen_on_all<B: CaptureBackend + 'static>(mut backend: B) -> Result<Box<dyn Stream<Item = Result<Packet>> + Unpin + Send>> {
    use std::collections::HashSet;

    use tokio::time::{interval, Duration};

    let mut merged_stream = StreamMap::new();

    // Create a channel to forward packets
    let (mut tx, rx) = mpsc::unbounded_channel();

    // Spawn task to manage stream and discover new devices
    tokio::spawn(async move {
        let mut new_devices = backend.listen_new_devices();
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
                res = new_devices.next() => {
                    let device = match res {
                        Some(Ok(device)) => device,
                        Some(Err(e)) => {
                            tracing::warn!("{:#?}", e);
                            continue
                        }
                        // backend down
                        None => {
                            drop(tx);
                            break
                        }
                    };
                    let device_name = device.name().to_owned();

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
    });

    Ok(Box::new(tokio_stream::wrappers::UnboundedReceiverStream::new(rx)))
}
