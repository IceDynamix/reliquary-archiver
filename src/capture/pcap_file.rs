use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::PathBuf;

use async_stream::stream;
use futures::stream::BoxStream;
use futures::{stream, FutureExt, Stream, StreamExt, TryFutureExt};
use notify::event::{CreateKind, EventAttributes, RemoveKind};
use notify::{Event, EventHandler, EventKind, RecursiveMode, Result as NotifyResult, Watcher};
use tokio::sync::mpsc::UnboundedSender;

use crate::capture::{CaptureBackend, CaptureDevice, CaptureError, Packet, PacketCapture};

#[derive(Debug)]
pub struct PcapFile {
    pub file: PathBuf,
}

impl PcapFile {
    pub(crate) fn new(file: PathBuf) -> Self {
        PcapFile { file }
    }
}

struct AsyncSyncWrapper(UnboundedSender<NotifyResult<Event>>);

impl EventHandler for AsyncSyncWrapper {
    fn handle_event(&mut self, event: NotifyResult<Event>) {
        self.0.send(event);
    }
}

impl CaptureBackend for PcapFile {
    type Device = PcapFile;

    fn list_devices(&mut self) -> crate::capture::Result<Vec<Self::Device>> {
        // HACK: due to CaptureBackend has no "start"
        // it cannot really start listening to the incoming events
        // TODO: uplift file opening to backend
        self.file
            .exists()
            .then(|| vec![PcapFile { file: self.file.clone() }])
            .ok_or_else(|| {
                CaptureError::Device(Box::new(std::io::Error::new(
                    ErrorKind::NotFound,
                    format!("file {:?} doesn't exist", self.file),
                )))
            })
    }

    fn listen_new_devices(&mut self) -> BoxStream<'_, crate::capture::Result<Self::Device>> {
        if !self.file.exists() {
            return stream::iter(vec![Err(CaptureError::Device(Box::new(std::io::Error::new(
                ErrorKind::NotFound,
                format!("file {:?} doesn't exist", self.file),
            ))))])
            .boxed();
        }
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<NotifyResult<Event>>();
        let mut watcher = match notify::recommended_watcher(AsyncSyncWrapper(tx)) {
            Ok(w) => w,
            Err(err) => return stream::iter(vec![Err(CaptureError::Device(Box::new(err)))]).boxed(),
        };

        if let Err(err) = watcher.watch(&self.file, RecursiveMode::NonRecursive) {
            return stream::iter(vec![Err(CaptureError::Device(Box::new(err)))]).boxed();
        }
        if !self.file.is_dir() {
            stream! {
                yield Ok(Event { kind: EventKind::Create(CreateKind::Any), paths: vec![self.file.clone()], attrs: EventAttributes::default() });
                while let Some(event) = rx.recv().await {
                    if let Ok(Event { kind: EventKind::Remove(RemoveKind::Any | RemoveKind::File), paths, .. }) = event {
                        break
                    }
                }
                drop(watcher);
            }.boxed()
        } else {
            stream! {
                while let Some(event) = rx.recv().await {
                    yield event;
                }
                drop(watcher);
            }.boxed()
        }
            .filter_map(async |res| match res {
                Ok(Event {
                       kind: EventKind::Create(CreateKind::File | CreateKind::Any),
                       paths,
                       ..
                   }) => Some(Ok(PcapFile { file: paths[0].clone() })), // TODO: test it out properly
                Ok(Event {
                       kind: EventKind::Remove(RemoveKind::File | RemoveKind::Any),
                       paths,
                       ..
                   }) => None,
                Err(err) => Some(Err(CaptureError::Device(Box::new(err)))),
                _ => None,
            })
            .boxed()
    }
}

impl CaptureDevice for PcapFile {
    type Capture = PcapFile;

    fn name(&self) -> &str {
        self.file.to_str().unwrap()
    }

    fn create_capture(&self) -> crate::capture::Result<Self::Capture> {
        Ok(PcapFile { file: self.file.clone() })
    }
}

impl PacketCapture for PcapFile {
    fn capture_packets(self) -> crate::capture::Result<impl Stream<Item = crate::capture::Result<Packet>> + Unpin + Send> {
        let source_id = {
            let mut hasher = DefaultHasher::new();
            self.file.hash(&mut hasher);
            hasher.finish()
        };
        Ok(Box::pin(stream! {
            let capture = tokio::fs::File::open(self.file.clone())
                .map_err(Box::<_>::from)
                .and_then(async |file| pcaparse::unified::Reader::async_new(file).map_err(Box::<_>::from).await)
                .map_err(CaptureError::Device);

            match capture.await {
                Ok(mut stream) => {
                    while let Some(packet_result) = stream.async_next_packet().await {
                        yield packet_result
                            .map_err(|e| CaptureError::Capture { has_captured: true, error: Box::new(e)})
                            .map(|p| Packet {
                                source_id,
                                data: p.data.to_vec(),
                            });
                    }
                }
                Err(err) => {
                    yield Err(err);
                }
            }
        }))
    }
}
