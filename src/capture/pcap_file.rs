use std::error::Error;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::PathBuf;
use std::pin::Pin;

use async_stream::stream;
use tokio::io::AsyncRead;

use crate::capture::{BoxError, CaptureError, Packet, PacketSource, PacketStream};

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
            let reader: Result<Pin<Box<dyn AsyncRead + Send>>, BoxError> = if *self.file == *"-" {
                Ok(Box::pin(tokio::io::stdin()) as Pin<Box<dyn AsyncRead + Send>>)
            } else {
                tokio::fs::File::open(self.file.clone()).await.map(|f| Box::pin(f) as Pin<Box<dyn AsyncRead + Send>>).map_err(|e| Box::new(e) as Box<dyn Error + Send + Sync>)
            };
            let capture = match reader {
                Ok(reader) => {
                    pcaparse::unified::Reader::async_new(reader).await.map_err(|err| Box::new(err) as Box<dyn Error + Send + Sync>)
                }
                Err(err) => Err(err)
            };
            match capture {
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
                    yield Err(CaptureError::Capture(err));
                }
            }
        }))
    }
}
