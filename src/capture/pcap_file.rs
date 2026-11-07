use std::hash::{DefaultHasher, Hash, Hasher};
use std::io::ErrorKind;
use std::path::PathBuf;

use async_stream::stream;
use futures::stream::BoxStream;
use futures::{FutureExt, Stream, StreamExt, TryFutureExt, stream};
use pcaparse::PcapError;

use crate::capture::{CaptureBackend, CaptureDevice, CaptureError, Packet, PacketCapture, PacketSource, PacketStream};

#[derive(Debug)]
pub struct PcapFile {
    pub file: PathBuf,
}

impl PcapFile {
    pub(crate) fn new(file: PathBuf) -> Self {
        PcapFile { file }
    }
}

impl PacketSource for PcapFile {
    fn packets(self) -> crate::capture::Result<PacketStream> {
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
                        match packet_result {
                            Ok(packet) => yield Ok(Packet {
                                source_id,
                                data: packet.data.to_vec(),
                            }),
                            Err(PcapError::IoError(err)) if err.kind() == ErrorKind::UnexpectedEof => {
                                break;
                            }
                            Err(err) => yield Err(CaptureError::Capture { has_captured: true, error: Box::new(err)}),
                        }
                    }
                }
                Err(err) => {
                    yield Err(err);
                }
            }
        }))
    }
}
