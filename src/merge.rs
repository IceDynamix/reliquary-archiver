//! Merges what several capture sources decoded from the same traffic.
//!
//! The same packet is often seen by more than one source: pcap on several interfaces, or pktmon
//! reporting it once for every component it passes through. Each source decodes its copies with
//! its own sniffer, so a source that misses or mangles a packet only breaks itself, and
//! [`SourceMerger`] passes each message on the first time any source produces it.
//!
//! Messages are matched by content. A message the game really sends twice is still passed on
//! twice: every source sees both, and a copy only counts as new once its source has produced it
//! more often than it has been passed on.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::time::{Duration, Instant};

use reliquary::network::command::GameCommand;

/// How long a message is remembered after its latest copy. Sources see the same packet within
/// milliseconds of each other, this only has to outlast a source that's briefly behind.
const MESSAGE_TTL: Duration = Duration::from_secs(60);

/// How often forgotten messages are cleaned up
const PRUNE_INTERVAL: Duration = Duration::from_secs(10);

/// Identifies a message across sources
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MessageKey {
    HandshakeEstablished { conv_id: u32 },
    Disconnected,
    Command { conv_id: u32, hash: u64 },
}

impl MessageKey {
    pub fn command(conv_id: u32, command: &GameCommand) -> Self {
        let mut hasher = DefaultHasher::new();
        command.command_id.hash(&mut hasher);
        command.proto_header.hash(&mut hasher);
        command.proto_data.hash(&mut hasher);

        MessageKey::Command {
            conv_id,
            hash: hasher.finish(),
        }
    }
}

#[derive(Debug)]
struct Seen {
    /// How often this message has been passed on
    delivered: u32,
    /// How often each source has produced it
    per_source: HashMap<u64, u32>,
    last_seen: Instant,
}

#[derive(Debug)]
pub struct SourceMerger {
    seen: HashMap<MessageKey, Seen>,
    last_prune: Option<Instant>,
}

impl Default for SourceMerger {
    fn default() -> Self {
        Self::new()
    }
}

impl SourceMerger {
    pub fn new() -> Self {
        Self {
            seen: HashMap::new(),
            last_prune: None,
        }
    }

    /// Whether `source` producing `key` is news, rather than a copy of something another source
    /// already produced
    pub fn accept(&mut self, source: u64, key: MessageKey, now: Instant) -> bool {
        self.prune(now);

        let seen = self.seen.entry(key).or_insert_with(|| Seen {
            delivered: 0,
            per_source: HashMap::new(),
            last_seen: now,
        });
        seen.last_seen = now;

        let produced = seen.per_source.entry(source).or_default();
        *produced += 1;

        if *produced > seen.delivered {
            seen.delivered = *produced;
            true
        } else {
            false
        }
    }

    fn prune(&mut self, now: Instant) {
        if self
            .last_prune
            .is_some_and(|last| now.saturating_duration_since(last) < PRUNE_INTERVAL)
        {
            return;
        }

        self.last_prune = Some(now);
        self.seen
            .retain(|_, seen| now.saturating_duration_since(seen.last_seen) < MESSAGE_TTL);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: u64 = 1;
    const B: u64 = 2;
    const C: u64 = 3;

    fn command(n: u64) -> MessageKey {
        MessageKey::Command { conv_id: 7, hash: n }
    }

    #[test]
    fn first_copy_wins() {
        let now = Instant::now();
        let mut merger = SourceMerger::new();

        assert!(merger.accept(A, command(1), now));
        assert!(!merger.accept(B, command(1), now));
        assert!(!merger.accept(C, command(1), now));
    }

    #[test]
    fn repeated_messages_are_delivered_each_time() {
        let now = Instant::now();
        let mut merger = SourceMerger::new();

        // The game sends the same message twice, both sources see both
        assert!(merger.accept(A, command(1), now));
        assert!(!merger.accept(B, command(1), now));
        assert!(merger.accept(A, command(1), now));
        assert!(!merger.accept(B, command(1), now));
    }

    #[test]
    fn a_lagging_source_takes_over_where_the_other_stopped() {
        let now = Instant::now();
        let mut merger = SourceMerger::new();

        // A delivers 1 and 2, then misses a packet and stops decoding
        assert!(merger.accept(A, command(1), now));
        assert!(merger.accept(A, command(2), now));

        // B was behind, catches up on what A delivered, then carries on alone
        assert!(!merger.accept(B, command(1), now));
        assert!(!merger.accept(B, command(2), now));
        assert!(merger.accept(B, command(3), now));
        assert!(merger.accept(B, command(4), now));
    }

    #[test]
    fn copies_from_one_source_are_not_lost_to_another_sources_repeats() {
        let now = Instant::now();
        let mut merger = SourceMerger::new();

        // A sees the message twice before B sees it at all
        assert!(merger.accept(A, command(1), now));
        assert!(merger.accept(A, command(1), now));
        assert!(!merger.accept(B, command(1), now));
        assert!(!merger.accept(B, command(1), now));
        // Only a third copy from B is news
        assert!(merger.accept(B, command(1), now));
    }

    #[test]
    fn keys_are_independent() {
        let now = Instant::now();
        let mut merger = SourceMerger::new();

        assert!(merger.accept(A, MessageKey::HandshakeEstablished { conv_id: 7 }, now));
        assert!(merger.accept(A, MessageKey::HandshakeEstablished { conv_id: 8 }, now));
        assert!(merger.accept(A, command(1), now));
        assert!(merger.accept(A, MessageKey::Command { conv_id: 8, hash: 1 }, now));
    }

    #[test]
    fn messages_are_forgotten_after_a_while() {
        let start = Instant::now();
        let mut merger = SourceMerger::new();

        assert!(merger.accept(A, MessageKey::Disconnected, start));
        assert!(!merger.accept(B, MessageKey::Disconnected, start));

        // A later disconnect, long after the first one was forgotten
        let later = start + MESSAGE_TTL + PRUNE_INTERVAL;
        assert!(merger.accept(B, MessageKey::Disconnected, later));
        assert!(!merger.accept(A, MessageKey::Disconnected, later));
        assert!(merger.seen.len() == 1);
    }
}
