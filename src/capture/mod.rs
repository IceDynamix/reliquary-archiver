//! Packet sources.
//!
//! Everything that can produce game traffic implements [`PacketSource`], which opens into a
//! [`PacketStream`] of Ethernet frames. Callers describe what they want with a [`SourceSpec`]
//! and call [`open_source`]; which backend ends up serving it is decided here and nowhere else.
//!
//! Live backends only need to implement [`DeviceBackend`], [`DeviceSource`] takes care of
//! device discovery and merging. File based sources implement [`PacketSource`] directly.

use std::error::Error;
use std::fmt::{Debug, Display};
use std::path::PathBuf;

use futures::StreamExt;
use futures::stream::BoxStream;
use tokio::sync::mpsc;
use tokio_stream::StreamMap;
use tokio_stream::wrappers::{ReceiverStream, UnboundedReceiverStream};
use tracing::instrument;

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

#[cfg(feature = "pcap")]
pub fn normalize_offline_pcap_payload(linktype: ::pcap::Linktype, payload: &[u8]) -> std::result::Result<Vec<u8>, String> {
    fn frame_ip_payload(payload: &[u8], ethertype: u16) -> Vec<u8> {
        // Use a synthetic Ethernet header so the downstream sniffer can parse RAW/IP captures.
        let mut framed = Vec::with_capacity(14 + payload.len());
        framed.extend_from_slice(&[0u8; 12]);
        framed.extend_from_slice(&ethertype.to_be_bytes());
        framed.extend_from_slice(payload);
        framed
    }

    let link_name = linktype.get_name().ok();
    let is_ethernet = linktype == ::pcap::Linktype::ETHERNET || link_name.as_deref() == Some("EN10MB");
    let is_ipv4 = linktype == ::pcap::Linktype::IPV4 || link_name.as_deref() == Some("IPV4");
    let is_ipv6 = linktype == ::pcap::Linktype::IPV6 || link_name.as_deref() == Some("IPV6");
    let is_raw_ip = linktype == ::pcap::Linktype::RAW || linktype.0 == 12 || link_name.as_deref() == Some("RAW");

    if is_ethernet {
        return Ok(payload.to_vec());
    }

    if payload.is_empty() {
        return Err("packet payload is empty".to_string());
    }

    if is_ipv4 {
        return Ok(frame_ip_payload(payload, 0x0800));
    }

    if is_ipv6 {
        return Ok(frame_ip_payload(payload, 0x86DD));
    }

    if is_raw_ip {
        let version = payload[0] >> 4;
        return match version {
            4 => Ok(frame_ip_payload(payload, 0x0800)),
            6 => Ok(frame_ip_payload(payload, 0x86DD)),
            _ => Err(format!("RAW packet does not start with IPv4/IPv6 header (version={version})")),
        };
    }

    let link_name = link_name.unwrap_or_else(|| "UNKNOWN".to_string());
    Err(format!("unsupported pcap datalink {link_name} ({})", linktype.0))
}

type BoxError = Box<dyn Error + Send + Sync>;

#[derive(Debug)]
pub enum CaptureError {
    /// The device or file could not be opened
    Device(BoxError),
    /// The packet filter could not be applied
    Filter(BoxError),
    /// Reading packets failed
    Capture(BoxError),
}

impl Display for CaptureError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CaptureError::Device(e) => write!(f, "could not open capture source: {e}"),
            CaptureError::Filter(e) => write!(f, "could not apply capture filter: {e}"),
            CaptureError::Capture(e) => write!(f, "capture failed: {e}"),
        }
    }
}

impl Error for CaptureError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            CaptureError::Device(e) | CaptureError::Filter(e) | CaptureError::Capture(e) => Some(e.as_ref()),
        }
    }
}

pub type Result<T> = std::result::Result<T, CaptureError>;

/// Represents a captured network packet (an Ethernet frame)
#[derive(Debug, Clone)]
pub struct Packet {
    pub source_id: u64,
    pub data: Vec<u8>,
}

/// A stream of captured packets. Ends when the source is exhausted, which never happens for live captures.
pub type PacketStream = BoxStream<'static, Result<Packet>>;

/// Anything that can produce game traffic
pub trait PacketSource: Send + 'static {
    /// Start reading packets from this source
    fn open(self: Box<Self>) -> Result<PacketStream>;
}

/// Describes where packets should be read from
#[derive(Debug, Clone)]
pub enum SourceSpec {
    /// Capture live traffic from the network
    Live,
    /// Read a recorded .pcap file
    #[cfg(feature = "pcap")]
    PcapFile(PathBuf),
    /// Read a recorded .etl file
    #[cfg(feature = "pktmon")]
    EtlFile(PathBuf),
}

impl SourceSpec {
    pub fn is_live(&self) -> bool {
        matches!(self, SourceSpec::Live)
    }
}

/// Open the packet source described by `spec`
pub fn open_source(spec: &SourceSpec) -> Result<PacketStream> {
    let source: Box<dyn PacketSource> = match spec {
        SourceSpec::Live => live_source(),
        #[cfg(feature = "pcap")]
        SourceSpec::PcapFile(path) => Box::new(pcap::PcapFileSource::new(path.clone())),
        #[cfg(feature = "pktmon")]
        SourceSpec::EtlFile(path) => Box::new(pktmon::EtlFileSource::new(path.clone())),
    };

    source.open()
}

fn live_source() -> Box<dyn PacketSource> {
    #[cfg(feature = "pcap")]
    {
        Box::new(DeviceSource(pcap::PcapBackend))
    }

    #[cfg(all(not(feature = "pcap"), feature = "pktmon"))]
    {
        Box::new(DeviceSource(pktmon::PktmonBackend))
    }
}

/// A live capture backend that reads from one or more network devices
pub trait DeviceBackend: Send + 'static {
    type Device: Send;

    /// List all available capture devices
    fn list_devices(&self) -> Result<Vec<Self::Device>>;

    /// Get the name of the device
    fn device_name(device: &Self::Device) -> String;

    /// Start capturing packets on the device
    fn open_device(device: &Self::Device) -> Result<PacketStream>;
}

/// Captures from all devices of a [`DeviceBackend`], picking up devices that appear later
pub struct DeviceSource<B>(pub B);

impl<B: DeviceBackend> PacketSource for DeviceSource<B> {
    #[instrument(skip_all)]
    fn open(self: Box<Self>) -> Result<PacketStream> {
        use tokio::time::{Duration, interval};

        let backend = self.0;

        if backend.list_devices()?.is_empty() {
            tracing::warn!("Could not find any network devices");
        }

        // Create a channel to forward packets
        let (tx, rx) = mpsc::unbounded_channel();

        // Spawn task to manage stream and discover new devices
        tokio::spawn(async move {
            let mut merged_stream = StreamMap::new();
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
                    _ = check_interval.tick() => add_new_devices(&backend, &mut merged_stream),
                }
            }
        });

        Ok(UnboundedReceiverStream::new(rx).boxed())
    }
}

fn add_new_devices<B: DeviceBackend>(backend: &B, streams: &mut StreamMap<String, PacketStream>) {
    let devices = match backend.list_devices() {
        Ok(devices) => devices,
        Err(e) => {
            tracing::warn!("Failed to list devices during periodic check: {:#?}", e);
            return;
        }
    };

    for device in devices {
        let device_name = B::device_name(&device);
        if streams.contains_key(&device_name) {
            continue;
        }

        tracing::info!("Discovered new device: {}", device_name);

        match B::open_device(&device) {
            Ok(stream) => {
                streams.insert(device_name, stream);
            }
            Err(e) => {
                tracing::warn!("Capture initialization error on new device {}: {:#?}", device_name, e);
            }
        }
    }
}

/// Runs a blocking packet reader on a worker thread and exposes what it reads as a stream.
///
/// `read` is handed a callback to pass packets to, which returns `false` once nobody is listening anymore.
fn blocking_stream(read: impl FnOnce(&mut dyn FnMut(Packet) -> bool) + Send + 'static) -> PacketStream {
    let (tx, rx) = mpsc::channel(1024);

    tokio::task::spawn_blocking(move || read(&mut |packet| tx.blocking_send(Ok(packet)).is_ok()));

    ReceiverStream::new(rx).boxed()
}

/// Derives a stable source id for packets read from a file
fn file_source_id(path: &std::path::Path) -> u64 {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

#[cfg(all(test, feature = "pcap"))]
mod tests {
    use ::pcap::Linktype;

    use super::normalize_offline_pcap_payload;

    #[test]
    fn ethernet_frames_are_passed_through() {
        let frame = vec![1, 2, 3, 4];
        assert_eq!(normalize_offline_pcap_payload(Linktype::ETHERNET, &frame).unwrap(), frame);
    }

    #[test]
    fn raw_ip_packets_get_a_synthetic_ethernet_header() {
        let ipv4 = [0x45, 0, 0, 20];
        let framed = normalize_offline_pcap_payload(Linktype::RAW, &ipv4).unwrap();
        assert_eq!(framed.len(), 14 + ipv4.len());
        assert_eq!(&framed[12..14], &[0x08, 0x00]);
        assert_eq!(&framed[14..], &ipv4);

        let ipv6 = [0x60, 0, 0, 0];
        let framed = normalize_offline_pcap_payload(Linktype::RAW, &ipv6).unwrap();
        assert_eq!(&framed[12..14], &[0x86, 0xDD]);
    }

    #[test]
    fn unusable_packets_are_rejected() {
        assert!(normalize_offline_pcap_payload(Linktype::RAW, &[]).is_err());
        assert!(normalize_offline_pcap_payload(Linktype::RAW, &[0x10]).is_err());
        assert!(normalize_offline_pcap_payload(Linktype::NULL, &[0x45]).is_err());
    }
}
