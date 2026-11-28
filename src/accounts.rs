use std::collections::HashMap;

use reliquary::network::command::{GameCommand, command_id};
use reliquary_archiver::export::Exporter;
use reliquary_archiver::export::fribbels::{OptimizerEvent, OptimizerExporter};
use tokio::sync::broadcast;
use tracing::info;

// Not every build has a subscriber that looks at the uid of lifecycle events
#[allow(dead_code)]
#[derive(Debug, Clone)]
pub enum AccountEvent {
    Discovered { uid: u32 },
    Reconnected { uid: u32 },
    ExporterEvent { uid: u32, event: OptimizerEvent },
}

/// Manages multiple account exporters, one per conversation.
///
/// Each exporter knows the uid of the account it belongs to, this keeps which conversation each
/// account is on now.
pub struct MultiAccountManager {
    /// All exporters indexed by conversation ID
    exporters: HashMap<u32, OptimizerExporter>,

    /// The conversation each account logged in on last
    uid_to_conv: HashMap<u32, u32>,

    /// The account that logged in most recently
    latest_uid: Option<u32>,

    /// Unified event channel for all account lifecycle and exporter events
    event_tx: broadcast::Sender<AccountEvent>,
}

impl MultiAccountManager {
    pub fn new() -> Self {
        let (event_tx, _) = broadcast::channel(100);
        Self {
            exporters: HashMap::new(),
            uid_to_conv: HashMap::new(),
            latest_uid: None,
            event_tx,
        }
    }

    /// Subscribe to all account events (discoveries, reconnections, exporter events)
    #[cfg(feature = "stream")]
    pub fn subscribe(&self) -> broadcast::Receiver<AccountEvent> {
        self.event_tx.subscribe()
    }

    /// Hands a command to the exporter of its conversation and passes on the events it makes.
    /// Returns the account's uid when the command is the one that says which account logged in.
    pub fn read_command(&mut self, conv_id: u32, command: GameCommand) -> Option<u32> {
        let is_login = command.command_id == command_id::PlayerGetTokenScRsp;

        let exporter = self.exporters.entry(conv_id).or_default();
        let events = exporter.read_command(command);
        let uid = exporter.uid();

        let logged_in = uid.filter(|_| is_login);
        if let Some(uid) = logged_in {
            self.latest_uid = Some(uid);
            self.register_uid(conv_id, uid);
        }

        if let Some(uid) = uid {
            for event in events {
                self.event_tx.send(AccountEvent::ExporterEvent { uid, event }).ok();
            }
        }

        logged_in
    }

    /// Register a UID for a conversation. If the UID was previously associated
    /// with a different conversation, delete the old exporter and emit reconnection event.
    fn register_uid(&mut self, conv_id: u32, uid: u32) {
        // Another account that logged in on this conversation before has been replaced by this one
        self.uid_to_conv.retain(|&other, &mut conv| other == uid || conv != conv_id);
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

        self.event_tx.send(event).ok();
    }

    /// The exporter of an account
    pub fn account_exporter(&self, uid: u32) -> Option<&OptimizerExporter> {
        self.exporters.get(self.uid_to_conv.get(&uid)?)
    }

    /// The exporter of the account that logged in most recently
    pub fn latest_exporter(&self) -> Option<&OptimizerExporter> {
        self.account_exporter(self.latest_uid?)
    }
}
