use std::collections::HashMap;
use std::sync::Arc;

use futures::lock::Mutex;
use reliquary_archiver::export::fribbels::{OptimizerEvent, OptimizerExporter};
use tokio::sync::broadcast;
#[cfg(feature = "stream")]
use tokio::task::JoinHandle;
use tracing::info;
#[cfg(feature = "stream")]
use tracing::warn;

// Not every build has a subscriber that looks at the uid of lifecycle events
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AccountEvent {
    Discovered { uid: u32 },
    Reconnected { uid: u32 },
    ExporterEvent { uid: u32, event: OptimizerEvent },
}

/// Manages multiple account exporters, one per conversation.
/// Maps conv_id -> exporter and uid -> conv_id for account tracking.
pub struct MultiAccountManager {
    /// All exporters indexed by conversation ID
    exporters: HashMap<u32, Arc<Mutex<OptimizerExporter>>>,

    /// Track which conversation belongs to which account UID
    uid_to_conv: HashMap<u32, u32>,

    /// Unified event channel for all account lifecycle and exporter events
    event_tx: broadcast::Sender<AccountEvent>,

    /// Task handles for exporter event forwarding (internal management)
    #[cfg(feature = "stream")]
    subscription_tasks: HashMap<u32, JoinHandle<()>>,
}

impl MultiAccountManager {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            exporters: HashMap::new(),
            uid_to_conv: HashMap::new(),
            event_tx,
            #[cfg(feature = "stream")]
            subscription_tasks: HashMap::new(),
        }
    }

    /// Subscribe to all account events (discoveries, reconnections, exporter events)
    #[cfg(feature = "stream")]
    pub fn subscribe(&self) -> broadcast::Receiver<AccountEvent> {
        self.event_tx.subscribe()
    }

    /// Get or create an exporter for a conversation ID
    pub fn get_or_create_exporter(&mut self, conv_id: u32) -> Arc<Mutex<OptimizerExporter>> {
        self.exporters
            .entry(conv_id)
            .or_insert_with(|| Arc::new(Mutex::new(OptimizerExporter::new())))
            .clone()
    }

    /// Register a UID for a conversation. If the UID was previously associated
    /// with a different conversation, delete the old exporter and emit reconnection event.
    pub fn register_uid(&mut self, conv_id: u32, uid: u32) {
        let event = match self.uid_to_conv.insert(uid, conv_id) {
            // Same conv_id, same UID - no-op
            Some(old_conv_id) if old_conv_id == conv_id => return,
            Some(old_conv_id) => {
                // Same UID reconnecting on a different conversation - delete old exporter
                info!(
                    uid,
                    old_conv = old_conv_id,
                    new_conv = conv_id,
                    "account reconnected, replacing old exporter"
                );
                self.exporters.remove(&old_conv_id);
                AccountEvent::Reconnected { uid }
            }
            None => {
                info!(uid, conv_id, "new account discovered");
                AccountEvent::Discovered { uid }
            }
        };

        #[cfg(feature = "stream")]
        if let Some(exporter) = self.get_account_exporter(uid) {
            self.spawn_exporter_subscription(uid, exporter);
        }

        self.event_tx.send(event).ok();
    }

    /// Get the exporter for a specific UID
    pub fn get_account_exporter(&self, uid: u32) -> Option<Arc<Mutex<OptimizerExporter>>> {
        self.uid_to_conv.get(&uid).and_then(|conv_id| self.exporters.get(conv_id)).cloned()
    }

    /// Get the exporter of any one account, for when the caller doesn't care which
    #[cfg(feature = "gui")]
    pub fn get_first_account_exporter(&self) -> Option<Arc<Mutex<OptimizerExporter>>> {
        self.uid_to_conv.values().find_map(|conv_id| self.exporters.get(conv_id)).cloned()
    }

    /// Spawn a task to forward events from an exporter's channel to the manager's unified event channel
    #[cfg(feature = "stream")]
    fn spawn_exporter_subscription(&mut self, uid: u32, exporter: Arc<Mutex<OptimizerExporter>>) {
        use reliquary_archiver::export::Exporter;
        use tokio::sync::broadcast::error::RecvError;

        let event_tx = self.event_tx.clone();

        let task = tokio::spawn(async move {
            let (initial_event, mut rx) = exporter.lock().await.subscribe();

            // Send initial event if available
            if let Some(event) = initial_event {
                event_tx.send(AccountEvent::ExporterEvent { uid, event }).ok();
            }

            // Forward all subsequent events
            loop {
                match rx.recv().await {
                    Ok(event) => {
                        if event_tx.send(AccountEvent::ExporterEvent { uid, event }).is_err() {
                            warn!(uid, "manager event channel closed");
                            break;
                        }
                    }
                    // Missing a few updates is recoverable for subscribers, losing the subscription is not
                    Err(RecvError::Lagged(skipped)) => {
                        warn!(uid, skipped, "exporter subscription lagged, some events were dropped");
                    }
                    Err(RecvError::Closed) => {
                        warn!(uid, "exporter subscription closed");
                        break;
                    }
                }
            }
        });

        // Store task handle (abort old one if exists)
        if let Some(old_task) = self.subscription_tasks.insert(uid, task) {
            old_task.abort();
        }
    }
}
