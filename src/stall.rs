//! Notices when a capture stops making progress.
//!
//! The game's traffic is KCP, which delivers commands strictly in order. The sniffer only watches,
//! so it can't ask for a retransmit: once the capture misses a segment (pktmon dropping packets
//! under load, for example), every later command waits on it forever. Packets keep arriving, but
//! nothing decodes, and the capture looks connected while it collects nothing.
//!
//! A healthy connection decodes commands regularly, even when the player is idle, because the
//! client and server exchange heartbeats. So game traffic that keeps arriving without a single
//! decoded command for a while means the capture has lost data.

use std::time::{Duration, Instant};

/// How long game traffic may arrive without any decoded command before the capture counts as stalled.
/// An idle connection can go about two minutes between heartbeats, so this leaves headroom above
/// that: a false alarm is worse than a late one.
pub const STALL_THRESHOLD: Duration = Duration::from_secs(3 * 60);

/// A change in whether the capture is making progress
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StallChange {
    /// Traffic kept arriving, but nothing could be decoded from it for [`STALL_THRESHOLD`]
    Stalled,
    /// Commands are being decoded again after a stall
    Recovered,
}

#[derive(Debug)]
pub struct StallDetector {
    threshold: Duration,
    /// When a command was last decoded, or the connection was established.
    /// `None` until then, since traffic from a connection that was already running when the
    /// capture started never decodes, and that's not a stall.
    last_progress: Option<Instant>,
    stalled: bool,
}

impl Default for StallDetector {
    fn default() -> Self {
        Self::new(STALL_THRESHOLD)
    }
}

impl StallDetector {
    pub fn new(threshold: Duration) -> Self {
        Self {
            threshold,
            last_progress: None,
            stalled: false,
        }
    }

    /// A connection was established, which starts the clock
    pub fn connection_established(&mut self, now: Instant) -> Option<StallChange> {
        self.progress(now)
    }

    /// A command was decoded
    pub fn command_decoded(&mut self, now: Instant) -> Option<StallChange> {
        self.progress(now)
    }

    /// The connection ended, there's nothing left to stall
    pub fn disconnected(&mut self) -> Option<StallChange> {
        self.last_progress = None;
        std::mem::take(&mut self.stalled).then_some(StallChange::Recovered)
    }

    /// Game traffic arrived. Reports [`StallChange::Stalled`] once it has gone on for too long
    /// without progress.
    pub fn traffic(&mut self, now: Instant) -> Option<StallChange> {
        let last_progress = self.last_progress?;

        if !self.stalled && now.saturating_duration_since(last_progress) >= self.threshold {
            self.stalled = true;
            return Some(StallChange::Stalled);
        }

        None
    }

    fn progress(&mut self, now: Instant) -> Option<StallChange> {
        self.last_progress = Some(now);
        std::mem::take(&mut self.stalled).then_some(StallChange::Recovered)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const THRESHOLD: Duration = Duration::from_secs(30);

    fn secs(start: Instant, s: u64) -> Instant {
        start + Duration::from_secs(s)
    }

    #[test]
    fn traffic_before_any_connection_never_stalls() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);

        assert_eq!(detector.traffic(start), None);
        assert_eq!(detector.traffic(secs(start, 600)), None);
    }

    #[test]
    fn steady_commands_never_stall() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);
        detector.connection_established(start);

        for s in (5..300).step_by(5) {
            assert_eq!(detector.traffic(secs(start, s)), None);
            assert_eq!(detector.command_decoded(secs(start, s)), None);
        }
    }

    #[test]
    fn traffic_without_commands_stalls_once() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);
        detector.connection_established(start);
        detector.command_decoded(secs(start, 1));

        assert_eq!(detector.traffic(secs(start, 30)), None);
        assert_eq!(detector.traffic(secs(start, 31)), Some(StallChange::Stalled));
        assert_eq!(detector.traffic(secs(start, 60)), None);
    }

    #[test]
    fn silence_is_not_a_stall() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);
        detector.connection_established(start);

        // Nothing arrived in between, so there's no traffic that failed to decode
        assert_eq!(detector.command_decoded(secs(start, 120)), None);
    }

    #[test]
    fn handshake_without_commands_stalls() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);
        detector.connection_established(start);

        assert_eq!(detector.traffic(secs(start, 30)), Some(StallChange::Stalled));
    }

    #[test]
    fn new_connection_recovers() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);
        detector.connection_established(start);
        detector.traffic(secs(start, 30));

        assert_eq!(detector.connection_established(secs(start, 40)), Some(StallChange::Recovered));
        assert_eq!(detector.traffic(secs(start, 50)), None);
    }

    #[test]
    fn decoded_command_recovers() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);
        detector.connection_established(start);
        detector.traffic(secs(start, 30));

        assert_eq!(detector.command_decoded(secs(start, 35)), Some(StallChange::Recovered));
    }

    #[test]
    fn disconnect_recovers_and_disarms() {
        let start = Instant::now();
        let mut detector = StallDetector::new(THRESHOLD);
        detector.connection_established(start);
        detector.traffic(secs(start, 30));

        assert_eq!(detector.disconnected(), Some(StallChange::Recovered));
        assert_eq!(detector.traffic(secs(start, 600)), None);
    }
}
