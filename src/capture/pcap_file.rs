use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;

use async_stream::stream;
use futures::TryFutureExt;

use crate::capture::{CaptureError, Packet, PacketSource, PacketStream};

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
    fn open(self: Box<Self>) -> crate::capture::Result<PacketStream> {
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
                            .map_err(|e| CaptureError::Capture (Box::new(e)))
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
