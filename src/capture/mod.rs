//! Packet sources.
//!
//! Everything that can produce game traffic implements [`PacketSource`], which opens into a
//! [`PacketStream`] of Ethernet frames. Callers describe what they want with a [`SourceSpec`]
//! (a [`Tap`] that runs until stopped, or a [`Replay`] of a recording) and open it; which
//! backend ends up serving it is decided here and nowhere else.
//!
//! Live backends only need to implement [`DeviceBackend`], [`DeviceSource`] takes care of
//! device discovery and merging. File based sources implement [`PacketSource`] directly.

use std::collections::HashMap;
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
pub mod pipe;

#[cfg(feature = "pcap")]
pub mod pcap;

/// The UDP ports the game talks to its servers on
const GAME_PORTS: (u16, u16) = (23301, 23302);

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

/// A source that delivers packets as they happen, reopening itself whenever it fails
#[derive(Debug, Clone)]
pub enum Tap {
    /// Capture traffic from the network devices
    #[cfg(feature = "pcap")]
    Pcap,
    /// Capture traffic from the network devices
    #[cfg(feature = "pktmon")]
    // pcap is preferred when both backends are built in
    #[cfg_attr(feature = "pcap", allow(dead_code))]
    Pktmon,
    /// Read pcap data from a pipe (or `-` for stdin) as it's written to it
    ///
    /// Whether a pipe will keep delivering can't be known ahead of time (`cat file.pcap |` is a
    /// pipe too), so it's only reopened when it ends if it's a named pipe a new writer can connect to
    #[cfg(feature = "pcap")]
    PcapPipe(PathBuf),
}

/// A recorded capture, read once from start to end
#[derive(Debug, Clone)]
pub enum Replay {
    /// A recorded .pcap or .pcapng file
    #[cfg(feature = "pcap")]
    Pcap(PathBuf),
    /// A recorded .etl file
    #[cfg(feature = "pktmon")]
    Etl(PathBuf),
}

/// Describes where packets should be read from
#[derive(Debug, Clone)]
pub enum SourceSpec {
    Tap(Tap),
    Replay(Replay),
}

impl SourceSpec {
    /// Whether the source keeps delivering packets until it's stopped
    pub fn is_tap(&self) -> bool {
        matches!(self, SourceSpec::Tap(_))
    }

    /// Start reading packets from the source
    pub fn open(&self) -> Result<PacketStream> {
        match self {
            SourceSpec::Tap(tap) => Ok(tap.open()),
            SourceSpec::Replay(replay) => replay.open(),
        }
    }
}

impl From<Tap> for SourceSpec {
    fn from(tap: Tap) -> Self {
        SourceSpec::Tap(tap)
    }
}

impl From<Replay> for SourceSpec {
    fn from(replay: Replay) -> Self {
        SourceSpec::Replay(replay)
    }
}

impl Tap {
    /// Start reading packets from the source
    ///
    /// When the capture can't be opened the error is passed along and opening is retried a
    /// second later. When it ends it's reopened if [`Tap::reopen_delay`] says so, otherwise the
    /// stream ends too.
    pub fn open(&self) -> PacketStream {
        use tokio::time::{Duration, sleep};

        let tap = self.clone();
        async_stream::stream! {
            loop {
                match tap.open_once() {
                    Ok(mut packets) => {
                        while let Some(packet) = packets.next().await {
                            yield packet;
                        }

                        let Some(delay) = tap.reopen_delay() else {
                            tracing::info!("capture finished");
                            break;
                        };

                        if delay.is_zero() {
                            tracing::info!("pipe writer finished, waiting for the next one");
                        } else {
                            tracing::warn!("capture ended unexpectedly, reopening in {} seconds", delay.as_secs());
                            sleep(delay).await;
                        }
                    }
                    Err(e) => {
                        yield Err(e);
                        sleep(Duration::from_secs(1)).await;
                    }
                }
            }
        }
        .boxed()
    }

    /// How long to wait before reopening the tap after it ends, or `None` if it's done for good
    fn reopen_delay(&self) -> Option<std::time::Duration> {
        use std::time::Duration;

        match self {
            // Device captures shouldn't end, give whatever went wrong a moment to recover
            #[cfg(feature = "pcap")]
            Tap::Pcap => Some(Duration::from_secs(10)),
            #[cfg(feature = "pktmon")]
            Tap::Pktmon => Some(Duration::from_secs(10)),
            // Opening a named pipe waits for the next writer, so there's no need to hold off
            #[cfg(feature = "pcap")]
            Tap::PcapPipe(path) => pipe::is_named_pipe(path).then_some(Duration::ZERO),
        }
    }

    fn open_once(&self) -> Result<PacketStream> {
        let source: Box<dyn PacketSource> = match self {
            #[cfg(feature = "pcap")]
            Tap::Pcap => Box::new(DeviceSource(pcap::PcapBackend)),
            #[cfg(feature = "pktmon")]
            Tap::Pktmon => Box::new(DeviceSource(pktmon::PktmonBackend)),
            // libpcap reads a pipe as packets are written to it, and stdin for "-"
            #[cfg(feature = "pcap")]
            Tap::PcapPipe(path) => Box::new(pcap::PcapFileSource::new(path.clone())),
        };

        source.open()
    }
}

impl Replay {
    /// The extensions of the capture files that can be read
    #[cfg(feature = "gui")]
    pub fn file_extensions() -> Vec<&'static str> {
        let mut extensions = Vec::new();
        #[cfg(feature = "pcap")]
        extensions.extend(["pcap", "pcapng"]);
        #[cfg(feature = "pktmon")]
        extensions.push("etl");
        extensions
    }

    /// The recording at `path`, if its extension is one that can be read
    #[cfg(feature = "gui")]
    pub fn for_path(path: PathBuf) -> Option<Self> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        match extension.as_str() {
            #[cfg(feature = "pcap")]
            "pcap" | "pcapng" => Some(Replay::Pcap(path)),
            #[cfg(feature = "pktmon")]
            "etl" => Some(Replay::Etl(path)),
            _ => None,
        }
    }

    /// Start reading packets from the recording
    pub fn open(&self) -> Result<PacketStream> {
        let source: Box<dyn PacketSource> = match self {
            #[cfg(feature = "pcap")]
            Replay::Pcap(path) => Box::new(pcap::PcapFileSource::new(path.clone())),
            #[cfg(feature = "pktmon")]
            Replay::Etl(path) => Box::new(pktmon::EtlFileSource::new(path.clone())),
        };

        source.open()
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
            let mut errors = DiscoveryErrors::default();
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
                    _ = check_interval.tick() => add_new_devices(&backend, &mut merged_stream, &mut errors),
                }
            }
        });

        Ok(UnboundedReceiverStream::new(rx).boxed())
    }
}

/// The errors device discovery last ran into, so that one that keeps happening on every check
/// is only logged as a warning the first time
#[derive(Default)]
struct DiscoveryErrors {
    /// Why listing the devices failed
    list: Option<String>,
    /// Why each device that could not be opened failed, by device name
    open: HashMap<String, String>,
}

fn add_new_devices<B: DeviceBackend>(backend: &B, streams: &mut StreamMap<String, PacketStream>, errors: &mut DiscoveryErrors) {
    let devices = match backend.list_devices() {
        Ok(devices) => devices,
        Err(e) => {
            let error = e.to_string();
            if errors.list.as_ref() == Some(&error) {
                tracing::debug!(%error, "Failed to list devices (still failing)");
            } else {
                tracing::warn!(%error, "Failed to list devices");
                errors.list = Some(error);
            }
            return;
        }
    };

    if errors.list.take().is_some() {
        tracing::info!("Listing devices works again");
    }

    // Forget devices that are no longer listed, so that if they come back and fail again it gets logged
    errors
        .open
        .retain(|name, _| devices.iter().any(|device| B::device_name(device) == *name));

    for device in devices {
        let device_name = B::device_name(&device);
        if streams.contains_key(&device_name) {
            continue;
        }

        match B::open_device(&device) {
            Ok(stream) => {
                if errors.open.remove(&device_name).is_some() {
                    tracing::info!("Opened device that previously failed: {}", device_name);
                } else {
                    tracing::info!("Discovered new device: {}", device_name);
                }
                streams.insert(device_name, stream);
            }
            Err(e) => {
                let error = e.to_string();
                if errors.open.get(&device_name) == Some(&error) {
                    tracing::debug!(%error, "Could not open device {} (still failing)", device_name);
                } else {
                    tracing::warn!(%error, "Could not open device {}", device_name);
                    errors.open.insert(device_name, error);
                }
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

/// Derives a stable source id from whatever identifies the source, like a device name or a file path
fn source_id(key: impl std::hash::Hash) -> u64 {
    use std::hash::{DefaultHasher, Hasher};

    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    hasher.finish()
}
