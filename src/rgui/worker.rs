use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use futures::channel::mpsc;
use futures::lock::Mutex;
use futures::sink::SinkExt;
use futures::{FutureExt, Stream, StreamExt, stream};
use reliquary_archiver::export::fribbels::OptimizerEvent;
use tokio::sync::broadcast::error::RecvError;
use tracing::{error, info, instrument, warn};

use crate::accounts::{AccountEvent, MultiAccountManager};
use crate::capture::Tap;
use crate::session::{CaptureSession, SessionEvent};

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    /// A packet arrived from the network
    PacketReceived,
    /// What the capture session made of a packet
    Session(SessionEvent),
    /// An account logged in, for the first time or again on a new connection
    AccountLoggedIn {
        uid: u32,
    },
    ExportEvent {
        uid: u32,
        event: OptimizerEvent,
    },
}

/// Creates a new [`Stream`] that produces the items sent from a [`Future`]
/// to the [`mpsc::Sender`] provided to the closure.
///
/// This is a more ergonomic [`stream::unfold`], which allows you to go
/// from the "world of futures" to the "world of streams" by simply looping
/// and publishing to an async channel from inside a [`Future`].
pub fn stream_channel<T>(size: usize, f: impl AsyncFnOnce(mpsc::Sender<T>)) -> impl Stream<Item = T> {
    let (sender, receiver) = mpsc::channel(size);

    let runner = stream::once(f(sender)).filter_map(|_| async { None });

    stream::select(receiver, runner)
}

#[instrument(skip_all)]
pub fn archiver_worker(manager: Arc<Mutex<MultiAccountManager>>, tap: Tap) -> impl Stream<Item = WorkerEvent> {
    stream_channel(100, |mut output: mpsc::Sender<WorkerEvent>| async move {
        tokio::spawn({
            let capture = tap_capture(manager.clone(), output.clone(), tap);

            async move {
                // Without this a panic would end the capture silently while the window stays open
                if AssertUnwindSafe(capture).catch_unwind().await.is_err() {
                    error!("the packet capture panicked and has stopped, please restart the application");
                }
            }
        });

        // Subscribe to manager's unified event channel
        let mut manager_rx = manager.lock().await.subscribe();

        // Forward events from manager's unified event channel
        loop {
            let event = match manager_rx.recv().await {
                Ok(event) => event,
                Err(RecvError::Lagged(skipped)) => {
                    warn!(skipped, "worker fell behind on account events, some were dropped");
                    continue;
                }
                Err(RecvError::Closed) => break,
            };

            let event = match event {
                AccountEvent::Discovered { uid } | AccountEvent::Reconnected { uid } => WorkerEvent::AccountLoggedIn { uid },
                AccountEvent::ExporterEvent { uid, event } => WorkerEvent::ExportEvent { uid, event },
            };

            output.send(event).await.ok();
        }
    })
}

#[instrument(skip_all)]
async fn tap_capture(manager: Arc<Mutex<MultiAccountManager>>, mut output: mpsc::Sender<WorkerEvent>, tap: Tap) {
    // The GUI keeps capturing for as long as it runs, a single bad packet shouldn't cost it a network interface
    let mut session = CaptureSession::new(manager, false);

    // The tap reopens itself whenever it fails, so this only ends if the tap is done for good
    let mut packets = tap.open();

    info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");

    while let Some(packet) = packets.next().await {
        let packet = match packet {
            Ok(packet) => packet,
            Err(e) => {
                warn!(%e);
                continue;
            }
        };

        output.send(WorkerEvent::PacketReceived).await.ok();

        for event in session.handle_packet(packet).await {
            output.send(WorkerEvent::Session(event)).await.ok();
        }
    }
}
