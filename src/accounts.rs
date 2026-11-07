use std::collections::HashMap;
use std::sync::Arc;

use futures::lock::Mutex;
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::{OptimizerEvent, OptimizerExporter};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tracing::{info, warn};

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
    subscription_tasks: HashMap<u32, JoinHandle<()>>,
}

impl MultiAccountManager {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            exporters: HashMap::new(),
            uid_to_conv: HashMap::new(),
            event_tx,
            subscription_tasks: HashMap::new(),
        }
    }

    /// Subscribe to all account events (discoveries, reconnections, exporter events)
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
    pub fn register_uid(&mut self, conv_id: u32, uid: u32) -> bool {
        let is_new_account = if let Some(&old_conv_id) = self.uid_to_conv.get(&uid) {
            if old_conv_id != conv_id {
                // Same UID reconnecting on a different conversation - delete old exporter
                info!(
                    uid,
                    old_conv = old_conv_id,
                    new_conv = conv_id,
                    "account reconnected, replacing old exporter"
                );
                self.exporters.remove(&old_conv_id);
                self.uid_to_conv.insert(uid, conv_id);

                // Get the new exporter and spawn subscription
                if let Some(exporter) = self.get_account_exporter(uid) {
                    self.spawn_exporter_subscription(uid, exporter);
                }

                // Emit reconnection event
                self.event_tx.send(AccountEvent::Reconnected { uid }).ok();

                false // Not a new account, it's a reconnection
            } else {
                // Same conv_id, same UID - no-op
                false
            }
        } else {
            // New UID discovered
            info!(uid, conv_id, "new account discovered");
            self.uid_to_conv.insert(uid, conv_id);

            // Get the exporter and spawn subscription
            if let Some(exporter) = self.get_account_exporter(uid) {
                self.spawn_exporter_subscription(uid, exporter);
            }

            // Emit discovery event
            self.event_tx.send(AccountEvent::Discovered { uid }).ok();

            true
        };

        is_new_account
    }

    /// Get the exporter for a specific UID
    pub fn get_account_exporter(&self, uid: u32) -> Option<Arc<Mutex<OptimizerExporter>>> {
        self.uid_to_conv.get(&uid).and_then(|conv_id| self.exporters.get(conv_id)).cloned()
    }

    /// Get the exporter of any one account, for when the caller doesn't care which
    pub fn get_first_account_exporter(&self) -> Option<Arc<Mutex<OptimizerExporter>>> {
        self.uid_to_conv.values().find_map(|conv_id| self.exporters.get(conv_id)).cloned()
    }

    /// Check if a conversation is still active (has an exporter)
    fn is_conv_active(&self, conv_id: u32) -> bool {
        self.exporters.contains_key(&conv_id)
    }

    /// Spawn a task to forward events from an exporter's channel to the manager's unified event channel
    fn spawn_exporter_subscription(&mut self, uid: u32, exporter: Arc<Mutex<OptimizerExporter>>) {
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
