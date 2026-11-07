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
use crate::capture::{self, SourceSpec};
use crate::session::{CaptureSession, SessionEvent};

#[derive(Debug, Clone)]
pub enum WorkerEvent {
    AccountDiscovered { uid: u32 },
    AccountReconnected { uid: u32 },
    ExportEvent { uid: u32, event: OptimizerEvent },
    Metric(SnifferMetric),
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
pub fn archiver_worker(manager: Arc<Mutex<MultiAccountManager>>) -> impl Stream<Item = WorkerEvent> {
    stream_channel(100, |mut output: mpsc::Sender<WorkerEvent>| async move {
        tokio::spawn({
            let capture = live_capture(manager.clone(), output.clone());

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

            match event {
                // The capture loop already reports every login as AccountDiscovered
                AccountEvent::Discovered { .. } => {}
                AccountEvent::Reconnected { uid } => {
                    output.send(WorkerEvent::AccountReconnected { uid }).await.ok();
                }
                AccountEvent::ExporterEvent { uid, event } => {
                    output.send(WorkerEvent::ExportEvent { uid, event }).await.ok();
                }
            }
        }
    })
}

#[derive(Debug, Clone)]
pub enum SnifferMetric {
    ConnectionEstablished,
    ConnectionDisconnected,
    NetworkPacketReceived,
    GameCommandsReceived(usize),
    DecryptionKeyMissing,
    NetworkError,
}

fn worker_event(event: SessionEvent) -> Option<WorkerEvent> {
    let metric = match event {
        SessionEvent::AccountSeen { uid } => return Some(WorkerEvent::AccountDiscovered { uid }),
        SessionEvent::LoginFinished => return None,
        SessionEvent::ConnectionEstablished => SnifferMetric::ConnectionEstablished,
        SessionEvent::Disconnected => SnifferMetric::ConnectionDisconnected,
        SessionEvent::PacketsDecoded(count) => SnifferMetric::GameCommandsReceived(count),
        SessionEvent::DecryptionKeyMissing => SnifferMetric::DecryptionKeyMissing,
        SessionEvent::DecodeError | SessionEvent::Fatal => SnifferMetric::NetworkError,
    };

    Some(WorkerEvent::Metric(metric))
}

#[instrument(skip_all)]
async fn live_capture(manager: Arc<Mutex<MultiAccountManager>>, mut output: mpsc::Sender<WorkerEvent>) {
    // The GUI keeps capturing for as long as it runs, a single bad packet shouldn't cost it a network interface
    let mut session = CaptureSession::new(manager, false);

    // Outer loop to restart capture when it exits
    loop {
        let mut packets = match capture::open_source(&SourceSpec::Live) {
            Ok(packets) => packets,
            Err(e) => {
                warn!(error = %e, "Failed to start packet capture, retrying in 1 second...");
                tokio::time::sleep(Duration::from_secs(1)).await;
                continue;
            }
        };

        info!("instructions: go to main menu screen and go to the \"Click to Start\" screen");

        while let Some(packet) = packets.next().await {
            let packet = match packet {
                Ok(packet) => packet,
                Err(e) => {
                    warn!(%e);
                    continue;
                }
            };

            output.send(WorkerEvent::Metric(SnifferMetric::NetworkPacketReceived)).await.ok();

            for event in session.handle_packet(packet).await {
                if let Some(event) = worker_event(event) {
                    output.send(event).await.ok();
                }
            }
        }

        warn!("live capture stream ended unexpectedly");
        info!("capture ended, restarting...");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
