use std::error::Error;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc};
use std::fmt::{Debug, Display};

use async_trait::async_trait;
use tokio::pin;
use tokio::sync::mpsc;
use futures::{Stream, StreamExt};
use tokio::task::JoinHandle;
use tokio_stream::wrappers::ReceiverStream;
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
    DeviceError(Box<dyn Error>),

    FilterError(Box<dyn Error>),
    CaptureError { has_captured: bool, error: Box<dyn Error> },
}

impl Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.source().unwrap().to_string())
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
    pub data: Vec<u8>,
}

/// Trait for implementing different packet capture backends
pub trait PacketCapture: Send {
    /// Start capturing packets and send them through the channel
    fn capture_packets(self) -> Result<impl Stream<Item = Result<Packet>> + Unpin>;
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
pub trait CaptureBackend {
    type Device: CaptureDevice;
    
    /// List all available capture devices
    fn list_devices(&self) -> Result<Vec<Self::Device>>;
}

/// Start capturing packets from all available devices using the specified backend
#[instrument(skip_all)]
pub fn listen_on_all<B: CaptureBackend + 'static>(
    backend: B,
) -> Result<impl Stream<Item = Result<Packet>> + Unpin> {
    // TODO: determine why pcap timeout is not working on linux, so that we can gracefully exit
    // TODO: add ctrl-c handler
    // #[cfg(not(target_os = "linux"))] {
    //     use std::sync::atomic::Ordering;
    //     use tracing::error;

    //     let abort_signal = abort_signal.clone();
    //     if let Err(e) = ctrlc::set_handler(move || {
    //         abort_signal.store(true, Ordering::Relaxed);
    //     }) {
    //         error!("Failed to set Ctrl-C handler: {}", e);
    //     }
    // }

    // let (tx, rx) = mpsc::channel(128);
    
    let devices = backend.list_devices()?;
    
    let mut merged_stream = StreamMap::new();

    for device in devices {
        // let tx = tx.clone();
        // let abort_signal = abort_signal.clone();

        // join_handles.push(tokio::spawn(async move {
        // tracing::debug!("Starting capture thread for device: {:?}", device);

        let mut capture = match device.create_capture() {
            Ok(c) => c,
            Err(e) => {
                tracing::debug!("Failed to create capture: {:#?}", e);
                return Err(e);
            }
        };

        let device_name = device.name().to_owned();
        
        match capture.capture_packets() {
            Ok(stream) => {
                merged_stream.insert(device_name, stream);
            }
            Err(e) => {
                tracing::warn!("Capture initialization error on device {}: {:#?}", device_name, e);
            }
        }
    }

    let merged_stream = StreamExt::map(merged_stream, |(_, item)| match item {
        Ok(p) => Ok(p),
        Err(e) => Err(e.also(|e| match e {
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
        })),
    });

    Ok(merged_stream)
}
